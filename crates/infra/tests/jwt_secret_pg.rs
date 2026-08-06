//! The self-minted JWT signing key, against a real Postgres.
//!
//! `DATABASE_URL` — skipped (passes) when unset so a machine without a database
//! is unaffected; a SET-but-unusable one panics, and a missing one in CI panics,
//! for the reason docs/lessons.md records as 「测试真空通过」: a skip that reads
//! like a pass is how a red test hides.
//!
//!   $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:5432/mica"
//!   cargo test -p mica-infra
use mica_infra::ensure_jwt_secret;
use sqlx::PgPool;

async fn pool() -> Option<PgPool> {
    let Ok(url) = std::env::var("DATABASE_URL") else {
        assert!(
            std::env::var("CI").is_err(),
            "DATABASE_URL is unset in CI — the postgres service block regressed; \
             these tests must not silently pass"
        );
        return None;
    };
    let db = PgPool::connect(&url)
        .await
        .expect("DATABASE_URL is set but the connection failed");
    // The schema is a PRECONDITION here, not this test's job.
    //
    // Calling `run_migrations` was the bug. Two mechanisms build this schema and
    // only one of them writes sqlx's `_sqlx_migrations` bookkeeping: CI applies
    // `migrations/*.sql` with a plain psql loop (see ci.yml — sqlx-cli would
    // cost minutes to compile), which leaves that table EMPTY. sqlx then saw no
    // record of anything, re-applied 0001 onto a live schema, and every CI run
    // died on `relation "users" already exists`. Against a dev database that WAS
    // sqlx-migrated it failed the other way — `VersionMismatch` the moment any
    // already-applied migration file got rewritten. Either way a JWT test went
    // red for reasons that have nothing to do with JWTs, and it was the ONLY
    // test mixing the two mechanisms.
    //
    // `run_migrations` keeps its real coverage where it belongs: ci.yml's
    // `container` job boots the actual api image against a fresh Postgres with
    // no psql loop, so the api migrates itself end to end.
    let migrated: bool =
        sqlx::query_scalar("SELECT to_regclass('public.server_secrets') IS NOT NULL")
            .fetch_one(&db)
            .await
            .expect("checking for the server_secrets table");
    assert!(
        migrated,
        "`server_secrets` is missing — DATABASE_URL points at an unmigrated database. \
         Apply them first: for f in migrations/*.sql; do psql \"$DATABASE_URL\" -f \"$f\"; done"
    );
    Some(db)
}

/// Start from "this instance has never minted one".
async fn clear(db: &PgPool) {
    sqlx::query("DELETE FROM server_secrets WHERE name = 'jwt_secret'")
        .execute(db)
        .await
        .unwrap();
}

/// ONE test, not four.
///
/// The subject is a SINGLETON row (`server_secrets.jwt_secret`), and every phase
/// below has to start from "never minted". As four separate `#[tokio::test]`s
/// they raced each other — cargo runs tests in parallel by default, one phase
/// deleted the row another had just written, and the concurrency phase failed
/// with two different keys. That looked exactly like a product bug and was not.
///
/// Serialising with `--test-threads=1` would work and would be a trap: nothing
/// in `just test` passes that flag, so the suite would go green here and red in
/// CI. Phases in one test are serial by construction.
#[tokio::test]
async fn the_signing_key_is_minted_once_and_kept() {
    let Some(db) = pool().await else { return };

    // ── an unconfigured instance mints one, and keeps it ──────────────────────
    clear(&db).await;
    let first = ensure_jwt_secret(&db, "").await.unwrap();
    // 32 bytes as hex — the same strength the docs used to ask operators to
    // produce by hand with `openssl rand -hex 32`.
    assert_eq!(first.len(), 64, "expected 32 bytes of hex: {first}");
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()));

    // The point of storing it: a restart must not invalidate every token this
    // instance already signed. A file in the api container could not do this —
    // it has no volume, so a container recreate would lose it.
    let second = ensure_jwt_secret(&db, "").await.unwrap();
    assert_eq!(first, second, "a second boot must reuse the stored key");

    // ── two instances booting together converge on ONE key ────────────────────
    clear(&db).await;
    // Whoever loses the insert must ADOPT the winner's value: overwriting it
    // would invalidate the tokens the winner already signed, and two replicas
    // would reject each other's.
    //
    // HONEST LIMIT: this does not reliably reproduce the race. On one runtime
    // `join!` lets the first future finish its INSERT before the second reads,
    // so the second usually takes the "already there" early return and never
    // mints. Verified by mutation — deleting the re-read in `ensure_jwt_secret`
    // leaves this GREEN. So it pins the OUTCOME (one key, whatever the
    // interleaving) and not the mechanism; the mechanism is carried by
    // `ON CONFLICT DO NOTHING` + the re-read, which are one line each.
    // A real interleaving test needs two pools and a barrier — more machinery
    // than a two-line invariant is worth.
    let (a, b) = tokio::join!(ensure_jwt_secret(&db, ""), ensure_jwt_secret(&db, ""));
    assert_eq!(a.unwrap(), b.unwrap());

    // ── an explicit secret wins, and is NOT shadowed in the database ──────────
    clear(&db).await;
    let configured = "an-operator-supplied-signing-key-long-enough";
    assert_eq!(
        ensure_jwt_secret(&db, configured).await.unwrap(),
        configured
    );
    let stored: Option<String> =
        sqlx::query_scalar("SELECT value FROM server_secrets WHERE name = 'jwt_secret'")
            .fetch_optional(&db)
            .await
            .unwrap();
    assert!(
        stored.is_none(),
        "explicit config must not be persisted — otherwise rotating JWT_SECRET          would silently do nothing on the next boot"
    );

    // ── whitespace is not configuration ───────────────────────────────────────
    // `JWT_SECRET="   "` reads as SET to `env::var`, and treating it as a key
    // would sign tokens with spaces.
    clear(&db).await;
    let minted = ensure_jwt_secret(&db, "   ").await.unwrap();
    assert_eq!(minted.len(), 64, "should have minted, not used the blanks");

    clear(&db).await;
}
