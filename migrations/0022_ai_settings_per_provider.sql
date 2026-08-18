-- One AI config PER PROVIDER, plus a marker for which one is in use.
--
-- 0021 stored a single row: one provider, one base URL, one key, one model. But
-- the settings dialog offers a provider DROPDOWN, and those two facts cannot
-- both be true. Switching provider rewrote the URL and the model while the key
-- stayed behind, so the screen ended up asserting two contradictory things at
-- once — a green "API key configured (ends 3027)" next to "the stored key
-- belongs to the previous provider". Both were accurate. The row was incoherent
-- by then: `protocol=openai`, a Zhipu base URL, a DeepSeek model id, and a
-- DeepSeek key, all in the same record, because nothing stopped it.
--
-- Patching the wording was the wrong move, twice. The mismatch is structural:
-- "is a key configured" is a question about A PROVIDER, and the schema could
-- only answer it about THE INSTANCE. So the row becomes one row per provider,
-- and switching means switching — every field follows, nothing is left over,
-- and the warning that explained the leftovers is deleted rather than reworded.
--
-- `provider_id` is the VENDOR (deepseek, zhipu, kimi, …) and is what the
-- dropdown selects; `protocol` is the wire format (openai | anthropic) and is
-- what the request code branches on. 0021 conflated them under the name
-- `provider`, which is why a Zhipu endpoint sat in a row labelled `openai`.

CREATE TABLE ai_provider_settings (
  provider_id text PRIMARY KEY,
  protocol    text NOT NULL,
  base_url    text NOT NULL,
  model       text NOT NULL DEFAULT '',
  api_key     text NOT NULL DEFAULT '',
  max_tokens  integer NOT NULL DEFAULT 2048,
  is_active   boolean NOT NULL DEFAULT false,
  updated_at  timestamptz NOT NULL DEFAULT now(),
  updated_by  uuid REFERENCES users(id) ON DELETE SET NULL
);

-- At most one active provider, enforced by the database rather than by every
-- writer remembering to clear the others first.
CREATE UNIQUE INDEX ai_provider_settings_single_active
  ON ai_provider_settings ((is_active))
  WHERE is_active;

-- Carry the single row over, attributing it by its endpoint — the base URL is
-- the only field that names a vendor unambiguously. Its `model` and `api_key`
-- come along untouched even though they may belong to a different vendor
-- (production's did): dropping them would be this migration guessing at what
-- the operator meant, and the settings screen now shows both under a named
-- provider, where a wrong one is visible instead of silent.
INSERT INTO ai_provider_settings (
  provider_id, protocol, base_url, model, api_key, max_tokens,
  is_active, updated_at, updated_by
)
SELECT
  CASE
    WHEN base_url LIKE '%deepseek.com%'   THEN 'deepseek'
    WHEN base_url LIKE '%bigmodel.cn%'    THEN 'zhipu'
    WHEN base_url LIKE '%z.ai%'           THEN 'zhipu'
    WHEN base_url LIKE '%moonshot.cn%'    THEN 'kimi'
    WHEN base_url LIKE '%api.openai.com%' THEN 'openai'
    WHEN base_url LIKE '%anthropic.com%'  THEN 'anthropic'
    ELSE 'custom'
  END,
  provider, base_url, model, api_key, max_tokens,
  true, updated_at, updated_by
FROM ai_settings;

DROP TABLE ai_settings;
