ALTER TABLE mimir_resources
  ADD COLUMN IF NOT EXISTS transcript_source TEXT
    CHECK(transcript_source IN ('youtube_transcript_api', 'youtubetranscript_dev', 'metadata_only')),
  ADD COLUMN IF NOT EXISTS transcript_mode TEXT
    CHECK(transcript_mode IN ('captions', 'asr', 'none')),
  ADD COLUMN IF NOT EXISTS transcript_chars INT;
