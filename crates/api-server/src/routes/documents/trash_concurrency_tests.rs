//! Real PostgreSQL races: the writer acquires the trigger's parent SHARE lock
//! first, then trash must wait and include its committed child (issue #59).
use super::{trash_one, trash_views_batch};
use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> Option<PgPool> {
  let Some(url) = mica_infra::test_database_url() else {
    return None;
  };
  Some(
    PgPool::connect(&url)
      .await
      .expect("DATABASE_URL must connect"),
  )
}

async fn seed(db: &PgPool) -> (Uuid, Uuid, Uuid) {
  let user = Uuid::new_v4();
  let ws = Uuid::new_v4();
  let root = Uuid::new_v4();
  sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,'T','x')")
    .bind(user)
    .bind(format!("{user}@trash-race.test"))
    .execute(db)
    .await
    .unwrap();
  sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'race',$2)")
    .bind(ws)
    .bind(user)
    .execute(db)
    .await
    .unwrap();
  sqlx::query(
    "INSERT INTO views(id,workspace_id,object_id,object_type,name,position,created_by) \
               VALUES($1,$2,$1,'folder','root','0',$3)",
  )
  .bind(root)
  .bind(ws)
  .bind(user)
  .execute(db)
  .await
  .unwrap();
  (ws, user, root)
}

async fn race(single: bool, nested: bool, moving: bool) {
  let Some(db) = pool().await else { return };
  let (ws, user, root) = seed(&db).await;
  let parent = if nested {
    let folder = Uuid::new_v4();
    sqlx::query("INSERT INTO views(id,workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
                 VALUES($1,$2,$3,$1,'folder','nested','0',$4)")
      .bind(folder).bind(ws).bind(root).bind(user).execute(&db).await.unwrap();
    folder
  } else {
    root
  };
  let child = Uuid::new_v4();
  let mut writer = db.begin().await.unwrap();
  let writer_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
    .fetch_one(&mut *writer)
    .await
    .unwrap();
  let insert = sqlx::query("INSERT INTO views(id,workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
               VALUES($1,$2,$3,$1,'document','child','0',$4)")
    .bind(child).bind(ws).bind(if moving { None } else { Some(parent) }).bind(user);
  if moving {
    // Move an already-committed page: the trash snapshot can see its old
    // top-level position while the writer holds the destination's SHARE lock.
    insert.execute(&db).await.unwrap();
  } else {
    insert.execute(&mut *writer).await.unwrap();
  }
  if moving {
    sqlx::query("UPDATE views SET parent_view_id = $1 WHERE id = $2")
      .bind(parent)
      .bind(child)
      .execute(&mut *writer)
      .await
      .unwrap();
  }
  let worker_db = db.clone();
  let trash = tokio::spawn(async move {
    if single {
      trash_one(&worker_db, ws, root).await.map(|_| Vec::new())
    } else {
      trash_views_batch(&worker_db, ws, &[root]).await
    }
  });

  // Observe a real blocked backend rather than assuming a sleep scheduled the
  // trash query. The old recursive UPDATE also waits here, but then misses the
  // child because its snapshot was taken before the writer committed.
  tokio::time::timeout(std::time::Duration::from_secs(10), async {
    loop {
      let blocked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))",
      )
      .bind(writer_pid)
      .fetch_one(&db)
      .await
      .unwrap();
      if blocked {
        break;
      }
      assert!(
        !trash.is_finished(),
        "trash must wait for the in-flight writer"
      );
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
  })
  .await
  .expect("trash must reach the parent's lock");
  writer.commit().await.unwrap();
  let touched = tokio::time::timeout(std::time::Duration::from_secs(10), trash)
    .await
    .unwrap()
    .unwrap()
    .unwrap();
  if !single {
    assert!(touched.contains(&root));
    assert!(
      touched.contains(&child),
      "batch response must include the new child"
    );
  }
  let live: i64 =
    sqlx::query_scalar("SELECT count(*) FROM views WHERE workspace_id = $1 AND NOT is_deleted")
      .bind(ws)
      .fetch_one(&db)
      .await
      .unwrap();
  assert_eq!(
    live, 0,
    "no committed child may remain live under a trashed ancestor"
  );
  let count: i64 = sqlx::query_scalar("SELECT count(*) FROM views WHERE workspace_id = $1")
    .bind(ws)
    .fetch_one(&db)
    .await
    .unwrap();
  assert_eq!(
    count,
    if nested { 3 } else { 2 },
    "trash preserves all views"
  );
  sqlx::query("DELETE FROM workspaces WHERE id = $1")
    .bind(ws)
    .execute(&db)
    .await
    .unwrap();
  sqlx::query("DELETE FROM users WHERE id = $1")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
}

#[tokio::test]
async fn single_trash_includes_an_insert_that_locked_the_parent_first() {
  race(true, false, false).await;
}

#[tokio::test]
async fn batch_trash_includes_an_insert_that_locked_the_parent_first() {
  race(false, false, false).await;
}

#[tokio::test]
async fn single_trash_includes_an_insert_under_a_nested_folder() {
  race(true, true, false).await;
}

#[tokio::test]
async fn batch_trash_includes_an_insert_under_a_nested_folder() {
  race(false, true, false).await;
}

#[tokio::test]
async fn single_trash_includes_a_move_that_locked_a_nested_parent_first() {
  race(true, true, true).await;
}

#[tokio::test]
async fn batch_trash_includes_a_move_that_locked_a_nested_parent_first() {
  race(false, true, true).await;
}

#[tokio::test]
async fn trash_does_not_follow_a_subtree_moved_out_while_waiting_for_its_lock() {
  let Some(db) = pool().await else { return };
  let (ws, user, root) = seed(&db).await;
  let folder = Uuid::new_v4();
  let page = Uuid::new_v4();
  sqlx::query(
    "INSERT INTO views(id,workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
     VALUES($1,$2,$3,$1,'folder','nested','0',$4),($5,$2,$1,$5,'document','leaf','0',$4)",
  )
  .bind(folder).bind(ws).bind(root).bind(user).bind(page)
  .execute(&db).await.unwrap();

  let mut writer = db.begin().await.unwrap();
  let writer_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
    .fetch_one(&mut *writer)
    .await
    .unwrap();
  sqlx::query("UPDATE views SET parent_view_id = NULL WHERE id = $1")
    .bind(folder)
    .execute(&mut *writer)
    .await
    .unwrap();
  let worker_db = db.clone();
  let trash = tokio::spawn(async move { trash_views_batch(&worker_db, ws, &[root]).await });
  tokio::time::timeout(std::time::Duration::from_secs(10), async {
    loop {
      let blocked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))",
      )
      .bind(writer_pid)
      .fetch_one(&db)
      .await
      .unwrap();
      if blocked {
        break;
      }
      assert!(
        !trash.is_finished(),
        "trash must reach the moving folder's row lock"
      );
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
  })
  .await
  .expect("trash must wait for move-out");
  writer.commit().await.unwrap();
  let touched = tokio::time::timeout(std::time::Duration::from_secs(10), trash)
    .await
    .unwrap()
    .unwrap()
    .unwrap();
  assert_eq!(
    touched,
    vec![root],
    "the moved-out folder and its page stay live"
  );
  let live: Vec<Uuid> =
    sqlx::query_scalar("SELECT id FROM views WHERE workspace_id = $1 AND NOT is_deleted")
      .bind(ws)
      .fetch_all(&db)
      .await
      .unwrap();
  assert_eq!(live.len(), 2);
  assert!(live.contains(&folder) && live.contains(&page));
  sqlx::query("DELETE FROM workspaces WHERE id = $1")
    .bind(ws)
    .execute(&db)
    .await
    .unwrap();
  sqlx::query("DELETE FROM users WHERE id = $1")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
}

#[tokio::test]
async fn overlapping_and_duplicate_trash_roots_change_each_view_once() {
  let Some(db) = pool().await else { return };
  let (ws, user, root) = seed(&db).await;
  let child = Uuid::new_v4();
  sqlx::query(
    "INSERT INTO views(id,workspace_id,parent_view_id,object_id,object_type,name,position,created_by) \
     VALUES($1,$2,$3,$1,'folder','nested','0',$4)",
  ).bind(child).bind(ws).bind(root).bind(user).execute(&db).await.unwrap();
  let touched = trash_views_batch(&db, ws, &[root, child, root])
    .await
    .unwrap();
  assert_eq!(touched.len(), 2);
  assert!(touched.contains(&root) && touched.contains(&child));
  assert!(
    trash_views_batch(&db, ws, &[root, child])
      .await
      .unwrap()
      .is_empty()
  );
  assert!(matches!(
    trash_one(&db, ws, root).await,
    Err(super::ApiError::NotFound)
  ));
  sqlx::query("DELETE FROM workspaces WHERE id = $1")
    .bind(ws)
    .execute(&db)
    .await
    .unwrap();
  sqlx::query("DELETE FROM users WHERE id = $1")
    .bind(user)
    .execute(&db)
    .await
    .unwrap();
}
