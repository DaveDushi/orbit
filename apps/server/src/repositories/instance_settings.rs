//! Settings of the whole instance that the root user changes in Admin: open registration and the outgoing mail
//! server. One row (`instance_settings.id = 1`, created by migration 0042).

use orbit_platform::{Database, Id, TimestampMillis};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use thiserror::Error;
use utoipa::ToSchema;

use crate::audit::{self, AuditOutcome};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SmtpSecurity {
    /// TLS from the first byte (usually port 465).
    Tls,
    /// Plain connection upgraded with STARTTLS (usually port 587). Refuses servers without STARTTLS.
    Starttls,
    /// No encryption. Only for a mail relay on the same host or private network.
    None,
}

impl SmtpSecurity {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::Starttls => "starttls",
            Self::None => "none",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "tls" => Some(Self::Tls),
            "starttls" => Some(Self::Starttls),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// The saved mail server. `password` is encrypted under the app key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub username: Option<String>,
    pub password: Option<Vec<u8>>,
    pub from_address: String,
    pub from_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceSettings {
    pub registration_open: bool,
    pub smtp: Option<SmtpSettings>,
}

/// What a save does with the stored SMTP password.
pub enum PasswordChange {
    Keep,
    Clear,
    /// Already encrypted.
    Set(Vec<u8>),
}

#[derive(Debug, Error)]
pub enum InstanceSettingsError {
    #[error("registration needs a configured mail server")]
    EmailNotConfigured,
    #[error("instance settings are unavailable")]
    Unavailable(#[from] sqlx::Error),
}

#[derive(Clone, Debug)]
pub struct InstanceSettingsRepository {
    database: Database,
}

impl InstanceSettingsRepository {
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn get(&self) -> Result<InstanceSettings, sqlx::Error> {
        let row = sqlx::query(
            "SELECT registration_open, smtp_host, smtp_port, smtp_security, smtp_username, smtp_password, \
             smtp_from_address, smtp_from_name FROM instance_settings WHERE id = 1",
        )
        .fetch_one(self.database.pool())
        .await?;
        let smtp = match (
            row.get::<Option<String>, _>("smtp_host"),
            row.get::<Option<i64>, _>("smtp_port"),
            row.get::<Option<String>, _>("smtp_security")
                .as_deref()
                .and_then(SmtpSecurity::parse),
            row.get::<Option<String>, _>("smtp_from_address"),
        ) {
            (Some(host), Some(port), Some(security), Some(from_address)) => Some(SmtpSettings {
                host,
                port: u16::try_from(port).unwrap_or(25),
                security,
                username: row.get("smtp_username"),
                password: row.get("smtp_password"),
                from_address,
                from_name: row.get("smtp_from_name"),
            }),
            _ => None,
        };
        Ok(InstanceSettings {
            registration_open: row.get::<i64, _>("registration_open") == 1,
            smtp,
        })
    }

    /// Opening registration needs a mail server: the sign-up link goes out by email.
    pub async fn set_registration_open(
        &self,
        actor_id: Id,
        open: bool,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<(), InstanceSettingsError> {
        let mut transaction = self.database.immediate_transaction().await?;
        if open {
            let configured = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM instance_settings WHERE id = 1 AND smtp_host IS NOT NULL",
            )
            .fetch_one(&mut *transaction)
            .await?;
            if configured == 0 {
                return Err(InstanceSettingsError::EmailNotConfigured);
            }
        }
        sqlx::query(
            "UPDATE instance_settings SET registration_open = ?, updated_at = ? WHERE id = 1",
        )
        .bind(i64::from(open))
        .bind(now.as_millis())
        .execute(&mut *transaction)
        .await?;
        audit::record_global(
            &mut transaction,
            Some(actor_id),
            if open {
                "instance.registration_opened"
            } else {
                "instance.registration_closed"
            },
            AuditOutcome::Success,
            "installation",
            None,
            request_id,
            serde_json::json!({}),
            now,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn save_smtp(
        &self,
        actor_id: Id,
        smtp: SmtpSettings,
        password: PasswordChange,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<(), sqlx::Error> {
        let mut transaction = self.database.immediate_transaction().await?;
        let password = match password {
            PasswordChange::Keep => {
                sqlx::query_scalar::<_, Option<Vec<u8>>>(
                    "SELECT smtp_password FROM instance_settings WHERE id = 1",
                )
                .fetch_one(&mut *transaction)
                .await?
            }
            PasswordChange::Clear => None,
            PasswordChange::Set(sealed) => Some(sealed),
        };
        sqlx::query(
            "UPDATE instance_settings SET smtp_host = ?, smtp_port = ?, smtp_security = ?, smtp_username = ?, \
             smtp_password = ?, smtp_from_address = ?, smtp_from_name = ?, updated_at = ? WHERE id = 1",
        )
        .bind(&smtp.host)
        .bind(i64::from(smtp.port))
        .bind(smtp.security.as_str())
        .bind(&smtp.username)
        .bind(password)
        .bind(&smtp.from_address)
        .bind(&smtp.from_name)
        .bind(now.as_millis())
        .execute(&mut *transaction)
        .await?;
        audit::record_global(
            &mut transaction,
            Some(actor_id),
            "instance.smtp_updated",
            AuditOutcome::Success,
            "installation",
            None,
            request_id,
            serde_json::json!({"host": smtp.host, "port": smtp.port}),
            now,
        )
        .await?;
        transaction.commit().await
    }

    /// Removes the mail server. Registration closes with it, since nobody could finish signing up.
    pub async fn clear_smtp(
        &self,
        actor_id: Id,
        request_id: &str,
        now: TimestampMillis,
    ) -> Result<(), sqlx::Error> {
        let mut transaction = self.database.immediate_transaction().await?;
        sqlx::query(
            "UPDATE instance_settings SET smtp_host = NULL, smtp_port = NULL, smtp_security = NULL, \
             smtp_username = NULL, smtp_password = NULL, smtp_from_address = NULL, smtp_from_name = NULL, \
             registration_open = 0, updated_at = ? WHERE id = 1",
        )
        .bind(now.as_millis())
        .execute(&mut *transaction)
        .await?;
        audit::record_global(
            &mut transaction,
            Some(actor_id),
            "instance.smtp_removed",
            AuditOutcome::Success,
            "installation",
            None,
            request_id,
            serde_json::json!({}),
            now,
        )
        .await?;
        transaction.commit().await
    }
}
