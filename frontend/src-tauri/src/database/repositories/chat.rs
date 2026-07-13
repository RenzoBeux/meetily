use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::database::models::ChatMessageModel;

pub struct ChatMessagesRepository;

impl ChatMessagesRepository {
    pub async fn add_message(
        pool: &SqlitePool,
        meeting_id: &str,
        role: &str,
        content: &str,
    ) -> Result<ChatMessageModel, sqlx::Error> {
        if role != "user" && role != "assistant" {
            return Err(sqlx::Error::Protocol(format!(
                "Invalid chat role: {}",
                role
            )));
        }

        let id = Uuid::new_v4().to_string();
        let created_at = Utc::now();

        sqlx::query(
            "INSERT INTO chat_messages (id, meeting_id, role, content, created_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(meeting_id)
        .bind(role)
        .bind(content)
        .bind(created_at)
        .execute(pool)
        .await?;

        Ok(ChatMessageModel {
            id,
            meeting_id: meeting_id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            created_at,
        })
    }

    pub async fn list_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<Vec<ChatMessageModel>, sqlx::Error> {
        sqlx::query_as::<_, ChatMessageModel>(
            "SELECT id, meeting_id, role, content, created_at \
             FROM chat_messages WHERE meeting_id = ? ORDER BY created_at ASC",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await
    }

    pub async fn clear_for_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM chat_messages WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn delete_message(pool: &SqlitePool, message_id: &str) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM chat_messages WHERE id = ?")
            .bind(message_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::test_support::migrated_pool;

    async fn insert_meeting(pool: &SqlitePool, id: &str) {
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, 'T', datetime('now'), datetime('now'))",
        )
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn add_list_and_clear_messages() {
        let pool = migrated_pool().await;
        insert_meeting(&pool, "m1").await;

        ChatMessagesRepository::add_message(&pool, "m1", "user", "hello")
            .await
            .unwrap();
        ChatMessagesRepository::add_message(&pool, "m1", "assistant", "hi there")
            .await
            .unwrap();

        let msgs = ChatMessagesRepository::list_for_meeting(&pool, "m1")
            .await
            .unwrap();
        assert_eq!(msgs.len(), 2);
        // Assert by set (two rapid inserts may share a created_at, making index
        // order unreliable).
        let contents: Vec<&str> = msgs.iter().map(|m| m.content.as_str()).collect();
        assert!(contents.contains(&"hello") && contents.contains(&"hi there"));

        let cleared = ChatMessagesRepository::clear_for_meeting(&pool, "m1")
            .await
            .unwrap();
        assert_eq!(cleared, 2);
        assert!(ChatMessagesRepository::list_for_meeting(&pool, "m1")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn invalid_role_is_rejected() {
        let pool = migrated_pool().await;
        insert_meeting(&pool, "m1").await;
        let result = ChatMessagesRepository::add_message(&pool, "m1", "system", "nope").await;
        assert!(result.is_err(), "an invalid chat role must be rejected");
    }
}
