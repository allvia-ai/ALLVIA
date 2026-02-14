use crate::llm_gateway::LLMClient;
use anyhow::Result;
use arrow::array::{Array, FixedSizeListArray, Float32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::{RecordBatch, RecordBatchIterator};
use futures::StreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{connect, Connection};
use std::sync::Arc;

pub struct MemoryStore {
    conn: Connection,
    table_name: String,
    llm: Arc<dyn LLMClient>,
}

impl MemoryStore {
    pub async fn new(uri: &str, llm: Arc<dyn LLMClient>) -> Result<Self> {
        let conn = connect(uri).execute().await?;

        Ok(Self {
            conn,
            table_name: "context_logs".to_string(),
            llm,
        })
    }

    #[allow(dead_code)]
    pub async fn init_table(&self) -> Result<()> {
        // Define Schema: id (utf8), text (utf8), vector (fixed_size_list<float32>[384]), metadata (utf8)
        // Note: For now, we rely on dynamic schema or explicit creation if not exists.
        // LanceDB often infers from data, but creating explicitly is safer.
        // However, lancedb-rs 0.4 might behave differently.
        // We will try to create if not exists using a dummy empty batch or check existence.

        // Simplified: We'll assume the table is created on first insert if not present
        // or we check self.conn.open_table(name).
        Ok(())
    }

    pub async fn add(&self, text: &str, metadata: serde_json::Value) -> Result<()> {
        let vector = self.llm.get_embedding(text).await?;

        // Create Arrow Arrays
        // 1. Context Text
        let text_array = StringArray::from(vec![text]);

        // 2. Vector (FixedSizeList)
        // OpenAI embedding size is 1536
        let dim = vector.len() as i32;

        let values = Float32Array::from(vector.clone());
        let field = Field::new("item", DataType::Float32, true);
        let vector_array = FixedSizeListArray::new(
            Arc::new(field),
            dim, // Embedding size
            Arc::new(values),
            None,
        );

        // Create Arrow Arrays
        // ... (existing code checks out)

        // 3. Metadata
        let meta_str = metadata.to_string();
        let meta_array = StringArray::from(vec![meta_str]);

        // Schema
        let schema = Arc::new(Schema::new(vec![
            Field::new("text", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim),
                false,
            ),
            Field::new("metadata", DataType::Utf8, true),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(text_array),
                Arc::new(vector_array),
                Arc::new(meta_array),
            ],
        )?;

        // Prepare data for potential create
        let batch_for_create = batch.clone();

        let _table = match self.conn.open_table(&self.table_name).execute().await {
            Ok(t) => {
                // Iterator for append
                let iterator = RecordBatchIterator::new(vec![Ok(batch)], schema.clone());
                t.add(iterator).execute().await?;

                // [Phase 25] Enforce Capacity Limit (1000 rows)
                // LanceDB does not have a simple count(), so we rely on periodic cleanup.
                // For now, we skip active trimming here to avoid blocking.
                // A background job or explicit cleanup call is preferred.
                t
            }
            Err(_) => {
                // Iterator for create
                let iterator = RecordBatchIterator::new(vec![Ok(batch_for_create)], schema);
                self.conn
                    .create_table(&self.table_name, iterator)
                    .execute()
                    .await?
            }
        };

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<(String, f64)>> {
        let vector = self.llm.get_embedding(query).await?;

        // Open Table
        let table = self.conn.open_table(&self.table_name).execute().await?;

        // Search
        let mut stream = table
            .query()
            .nearest_to(vector)?
            .limit(limit)
            .execute()
            .await?;

        let mut results = Vec::new();
        while let Some(batch_result) = stream.next().await {
            let batch = batch_result?;
            if let Some(text_col) = batch.column_by_name("text") {
                // Explicit cast to help inference
                let col: &dyn std::any::Any = text_col.as_any();
                if let Some(strings) = col.downcast_ref::<StringArray>() {
                    for i in 0..strings.len() {
                        results.push((strings.value(i).to_string(), 0.0));
                    }
                }
            }
        }

        Ok(results)
    }

    /// [Phase 28] Cleanup old entries - call from background scheduler
    /// Keeps only the most recent `max_rows` entries
    #[allow(dead_code)]
    pub async fn cleanup(&self, max_rows: usize) -> Result<usize> {
        // LanceDB doesn't have a simple DELETE with LIMIT, so we:
        // 1. Read all rows
        // 2. If count > max_rows, drop table and recreate with recent rows
        // This is a heavy operation, meant for background jobs only

        let table = match self.conn.open_table(&self.table_name).execute().await {
            Ok(t) => t,
            Err(_) => return Ok(0), // No table, nothing to cleanup
        };

        // Count approximate rows by reading all
        let mut stream = table.query().execute().await?;
        let mut all_texts = Vec::new();
        let mut all_vectors = Vec::new();
        let mut all_metadata = Vec::new();

        while let Some(batch_result) = stream.next().await {
            let batch = batch_result?;
            if let (Some(text_col), Some(vec_col), Some(meta_col)) = (
                batch.column_by_name("text"),
                batch.column_by_name("vector"),
                batch.column_by_name("metadata"),
            ) {
                let texts = text_col.as_any().downcast_ref::<StringArray>().unwrap();
                let metas = meta_col.as_any().downcast_ref::<StringArray>().unwrap();
                let vecs = vec_col
                    .as_any()
                    .downcast_ref::<FixedSizeListArray>()
                    .unwrap();

                for i in 0..texts.len() {
                    all_texts.push(texts.value(i).to_string());
                    all_metadata.push(metas.value(i).to_string());
                    // Extract vector values
                    let vec_array = vecs.value(i);
                    let float_arr = vec_array.as_any().downcast_ref::<Float32Array>().unwrap();
                    let vec: Vec<f32> = float_arr.iter().map(|v| v.unwrap_or(0.0)).collect();
                    all_vectors.push(vec);
                }
            }
        }

        let total = all_texts.len();
        if total <= max_rows {
            return Ok(0); // Nothing to cleanup
        }

        // Keep only most recent (last N entries)
        let to_remove = total - max_rows;
        let texts_to_keep = &all_texts[to_remove..];
        let vectors_to_keep = &all_vectors[to_remove..];
        let metadata_to_keep = &all_metadata[to_remove..];

        // Drop and recreate table (LanceDB limitation)
        self.conn.drop_table(&self.table_name).await?;

        // Recreate with kept data
        for (i, text) in texts_to_keep.iter().enumerate() {
            let meta: serde_json::Value =
                serde_json::from_str(&metadata_to_keep[i]).unwrap_or(serde_json::json!({}));

            // Use add() which will recreate table on first call
            // But we need the vector, so we bypass LLM
            let _ = self.add_with_vector(text, &vectors_to_keep[i], meta).await;
        }

        println!(
            "🧹 [Memory] Cleaned up {} old entries, kept {}",
            to_remove, max_rows
        );
        Ok(to_remove)
    }

    /// Add with pre-computed vector (for cleanup/migration)
    async fn add_with_vector(
        &self,
        text: &str,
        vector: &[f32],
        metadata: serde_json::Value,
    ) -> Result<()> {
        let dim = vector.len() as i32;
        let text_array = StringArray::from(vec![text]);
        let values = Float32Array::from(vector.to_vec());
        let field = Field::new("item", DataType::Float32, true);
        let vector_array = FixedSizeListArray::new(Arc::new(field), dim, Arc::new(values), None);
        let meta_str = metadata.to_string();
        let meta_array = StringArray::from(vec![meta_str]);

        let schema = Arc::new(Schema::new(vec![
            Field::new("text", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), dim),
                false,
            ),
            Field::new("metadata", DataType::Utf8, true),
        ]));

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(text_array),
                Arc::new(vector_array),
                Arc::new(meta_array),
            ],
        )?;

        let batch_for_create = batch.clone();
        let _ = match self.conn.open_table(&self.table_name).execute().await {
            Ok(t) => {
                let iterator = RecordBatchIterator::new(vec![Ok(batch)], schema.clone());
                t.add(iterator).execute().await?;
                t
            }
            Err(_) => {
                let iterator = RecordBatchIterator::new(vec![Ok(batch_for_create)], schema);
                self.conn
                    .create_table(&self.table_name, iterator)
                    .execute()
                    .await?
            }
        };

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_gateway::OpenAILLMClient;
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_memory_functionality() {
        let run_live = std::env::var("STEER_RUN_LIVE_EMBED_TEST")
            .ok()
            .map(|v| {
                matches!(
                    v.trim().to_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        if !run_live {
            // Default deterministic mode: skip live embedding API integration unless explicitly enabled.
            return;
        }

        if std::env::var("OPENAI_API_KEY").is_err() {
            dotenv::dotenv().ok();
        }

        if std::env::var("OPENAI_API_KEY").is_err() {
            // Skip if no key (CI friendly)
            return;
        }

        let temp_dir = std::env::temp_dir().join("steer_test_mem");
        if temp_dir.exists() {
            std::fs::remove_dir_all(&temp_dir).unwrap_or(());
        }
        let uri = temp_dir.to_str().unwrap();

        let llm = Arc::new(OpenAILLMClient::new().unwrap());
        let store = MemoryStore::new(uri, llm).await.unwrap();

        // 1. Add
        store
            .add("Steer is an AI Agent", json!({"source": "manual"}))
            .await
            .unwrap();

        // 2. Search
        let results = store.search("AI Agent", 1).await.unwrap();
        assert!(!results.is_empty());
        println!("Search Result: {:?}", results);
        assert!(results[0].0.contains("Steer"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
