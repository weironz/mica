\set ON_ERROR_STOP on
BEGIN;
CREATE SCHEMA mica_performance_round3;
SET LOCAL search_path=mica_performance_round3,public;
CREATE TABLE views (LIKE public.views INCLUDING ALL);
CREATE TABLE document_yrs_base (LIKE public.document_yrs_base INCLUDING ALL);
CREATE TABLE workspace_updates (LIKE public.workspace_updates INCLUDING ALL);
INSERT INTO views(id,workspace_id,parent_view_id,object_id,object_type,name,position,is_deleted,created_by,created_at,updated_at)
SELECT md5('v'||g)::uuid, md5('ws'||((g-1)/20000))::uuid,
 CASE WHEN g%3=0 THEN NULL ELSE md5('folder'||((g-1)/20000)||'-'||(g%100))::uuid END,
 md5('d'||g)::uuid,'document'::object_type,
 CASE WHEN g%1000=0 THEN 'Needle-' ELSE 'Page-' END||g,
 lpad((g%1000)::text,10,'0'),g%19=0,md5('actor')::uuid,
 now()-make_interval(secs=>g),now()-make_interval(secs=>g)
FROM generate_series(1,400000) AS g;
INSERT INTO workspace_updates(rid,workspace_id,document_id,actor_id,payload,created_at)
SELECT g,md5('ws'||(g%20))::uuid,md5('d'||(g%1000))::uuid,md5('actor')::uuid,decode(repeat('ab',128),'hex'),now()-make_interval(secs=>g)
FROM generate_series(1,200000) AS g;
ANALYZE views;
ANALYZE workspace_updates;
SELECT 'QUERY tree_flat';
EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON)
SELECT v.id,v.workspace_id,v.parent_view_id,v.object_id,v.object_type::text,v.name,v.icon,v.position,v.is_deleted,v.created_by,v.created_at,v.updated_at,octet_length(b.state)::bigint AS state_bytes
FROM views v LEFT JOIN document_yrs_base b ON false AND b.document_id=v.object_id
WHERE v.workspace_id=md5('ws0')::uuid AND v.is_deleted=false
ORDER BY v.parent_view_id NULLS FIRST,v.position,v.id;
SELECT 'QUERY tree_limit50';
EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON)
SELECT id,workspace_id,parent_view_id,object_id,object_type::text,name,position,is_deleted,created_at,updated_at
FROM views WHERE workspace_id=md5('ws0')::uuid AND is_deleted=false
ORDER BY parent_view_id NULLS FIRST,position,id LIMIT 50;
SELECT 'QUERY search_one_workspace';
EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON)
SELECT v.id,v.object_id,v.workspace_id,v.name,v.parent_view_id,v.object_type::text
FROM views v WHERE v.workspace_id=ANY(ARRAY[md5('ws0')::uuid]) AND v.is_deleted=false AND v.object_type='document'
AND (v.name ILIKE '%Needle%' ESCAPE '\' OR v.object_id=ANY(ARRAY[md5('d3')::uuid,md5('d5')::uuid]))
ORDER BY (v.name ILIKE '%Needle%' ESCAPE '\') DESC,(v.workspace_id=md5('ws0')::uuid) DESC,(v.object_type<>'document') DESC,(v.name ILIKE 'Needle' ESCAPE '\') DESC,(v.name ILIKE 'Needle%' ESCAPE '\') DESC,v.updated_at DESC LIMIT 50;
SELECT 'QUERY search_all_workspaces';
EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON)
SELECT v.id,v.object_id,v.workspace_id,v.name,v.parent_view_id,v.object_type::text
FROM views v WHERE v.workspace_id=ANY(ARRAY(SELECT md5('ws'||g)::uuid FROM generate_series(0,19) AS g)) AND v.is_deleted=false AND v.object_type='document'
AND (v.name ILIKE '%Needle%' ESCAPE '\' OR v.object_id=ANY(ARRAY[md5('d3')::uuid,md5('d5')::uuid]))
ORDER BY (v.name ILIKE '%Needle%' ESCAPE '\') DESC,(v.workspace_id=md5('ws0')::uuid) DESC,(v.object_type<>'document') DESC,(v.name ILIKE 'Needle' ESCAPE '\') DESC,(v.name ILIKE 'Needle%' ESCAPE '\') DESC,v.updated_at DESC LIMIT 50;
SELECT 'QUERY deleted_tree';
EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON)
SELECT id,name,updated_at FROM views WHERE workspace_id=md5('ws0')::uuid AND is_deleted=true ORDER BY updated_at DESC;
SELECT 'QUERY catch_up';
EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON)
SELECT rid,document_id,actor_id,payload FROM workspace_updates WHERE document_id=md5('d3')::uuid AND rid>100000 ORDER BY rid LIMIT 500;
CREATE INDEX benchmark_order ON views(workspace_id,parent_view_id NULLS FIRST,position,id) WHERE is_deleted=false;
ANALYZE views;
SELECT 'QUERY candidate_tree_limit50';
EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON)
SELECT id,workspace_id,parent_view_id,object_id,object_type::text,name,position,is_deleted,created_at,updated_at
FROM views WHERE workspace_id=md5('ws0')::uuid AND is_deleted=false
ORDER BY parent_view_id NULLS FIRST,position,id LIMIT 50;
ROLLBACK;
