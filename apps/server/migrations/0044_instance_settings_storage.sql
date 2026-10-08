-- Object storage (S3) for attachments and backups, and automatic backups. The root user changes them in
-- Admin → Storage. The bucket is saved when the endpoint, bucket and access key are all set (checked in code).
ALTER TABLE instance_settings ADD COLUMN s3_endpoint TEXT
    CHECK (s3_endpoint IS NULL OR length(s3_endpoint) BETWEEN 8 AND 2048);
ALTER TABLE instance_settings ADD COLUMN s3_region TEXT
    CHECK (s3_region IS NULL OR length(s3_region) BETWEEN 1 AND 64);
ALTER TABLE instance_settings ADD COLUMN s3_bucket TEXT
    CHECK (s3_bucket IS NULL OR length(s3_bucket) BETWEEN 3 AND 63);
ALTER TABLE instance_settings ADD COLUMN s3_prefix TEXT NOT NULL DEFAULT ''
    CHECK (length(s3_prefix) <= 512);
ALTER TABLE instance_settings ADD COLUMN s3_access_key_id TEXT
    CHECK (s3_access_key_id IS NULL OR length(s3_access_key_id) BETWEEN 1 AND 256);
-- ChaCha20-Poly1305 under the server app key (secret_box.rs), never plain text.
ALTER TABLE instance_settings ADD COLUMN s3_secret_access_key BLOB;
ALTER TABLE instance_settings ADD COLUMN s3_path_style INTEGER NOT NULL DEFAULT 0
    CHECK (s3_path_style IN (0, 1));
ALTER TABLE instance_settings ADD COLUMN attachments_in_s3 INTEGER NOT NULL DEFAULT 0
    CHECK (attachments_in_s3 IN (0, 1));
ALTER TABLE instance_settings ADD COLUMN backups_in_s3 INTEGER NOT NULL DEFAULT 0
    CHECK (backups_in_s3 IN (0, 1));
ALTER TABLE instance_settings ADD COLUMN backup_schedule TEXT NOT NULL DEFAULT 'off'
    CHECK (backup_schedule IN ('off', 'hourly', 'daily'));
