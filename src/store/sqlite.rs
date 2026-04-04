use crate::error::SpiderError;
use crate::item::Item;
use crate::store::Store;
use crate::store::database::{
    FieldColumn, FieldColumnType, FieldColumnValue, build_field_column, map_field_column_value,
    quote_identifier, validate_field_columns, validate_identifier,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{QueryBuilder, SqlitePool};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct Sqlite {
    path: PathBuf,
    table: String,
    field_columns: Vec<FieldColumn>,
    pool: Arc<Mutex<Option<SqlitePool>>>,
}

impl Sqlite {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            table: "items".to_string(),
            field_columns: Vec::new(),
            pool: Arc::new(Mutex::new(None)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn table(&self) -> &str {
        &self.table
    }

    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        self.table = table.into();
        self
    }

    pub fn with_field_column(
        mut self,
        field: impl Into<String>,
        name: impl Into<String>,
        column_type: FieldColumnType,
    ) -> Self {
        self.field_columns
            .push(build_field_column(field, name, column_type));
        self
    }

    async fn pool(&self) -> Result<SqlitePool, SpiderError> {
        {
            let guard = self.pool.lock().await;
            if let Some(pool) = guard.clone() {
                return Ok(pool);
            }
        }

        let pool = self.open_pool().await?;
        let mut guard = self.pool.lock().await;

        if let Some(existing) = guard.clone() {
            return Ok(existing);
        }

        *guard = Some(pool.clone());
        Ok(pool)
    }

    async fn open_pool(&self) -> Result<SqlitePool, SpiderError> {
        validate_identifier("sqlite", "table", &self.table)?;
        validate_field_columns("sqlite", &self.field_columns)?;
        ensure_parent_dir(&self.path).await?;

        let connect_options = sqlite_connect_options(&self.path)?;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(connect_options)
            .await
            .map_err(|error| {
                SpiderError::engine(format!("failed to open sqlite store database: {error}"))
            })?;

        let statement = build_create_table_statement(&self.table, &self.field_columns);
        sqlx::query(&statement)
            .execute(&pool)
            .await
            .map_err(|error| {
                SpiderError::engine(format!("failed to initialize sqlite store table: {error}"))
            })?;

        Ok(pool)
    }

    async fn insert_item(
        &self,
        pool: &SqlitePool,
        item: &Item,
        spider_name: &str,
    ) -> Result<(), SpiderError> {
        self.insert_items(pool, std::slice::from_ref(item), spider_name)
            .await
    }

    async fn insert_items(
        &self,
        pool: &SqlitePool,
        items: &[Item],
        spider_name: &str,
    ) -> Result<(), SpiderError> {
        if items.is_empty() {
            return Ok(());
        }

        let rows = items
            .iter()
            .map(|item| {
                let item_json = serde_json::to_string(&item.to_json()).map_err(|error| {
                    SpiderError::engine(format!(
                        "failed to serialize item for sqlite store: {error}"
                    ))
                })?;
                let mapped_values = self
                    .field_columns
                    .iter()
                    .map(|column| map_field_column_value("sqlite", item, column))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(normalize_sqlite_field_column_value)
                    .collect::<Result<Vec<_>, _>>()?;

                Ok::<_, SpiderError>((item_json, mapped_values))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut builder = QueryBuilder::new("INSERT INTO ");
        builder.push(quote_identifier(&self.table));
        builder.push(" (");

        {
            let mut columns = builder.separated(", ");
            columns.push(quote_identifier("spider_name"));
            columns.push(quote_identifier("item_json"));
            for column in &self.field_columns {
                columns.push(quote_identifier(&column.name));
            }
        }

        builder.push(") ");
        builder.push_values(rows, |mut row, (item_json, mapped_values)| {
            row.push_bind(spider_name.to_string());
            row.push_bind(item_json);
            for value in mapped_values {
                match value {
                    FieldColumnValue::Null => row.push("NULL"),
                    FieldColumnValue::Text(value) => row.push_bind(value),
                    FieldColumnValue::Integer(value) => row.push_bind(value),
                    FieldColumnValue::Real(value) => row.push_bind(value),
                    FieldColumnValue::Bool(value) => {
                        row.push_bind(if value { 1_i64 } else { 0_i64 })
                    }
                    FieldColumnValue::Json(_) => {
                        unreachable!(
                            "sqlite json values should be normalized into text before binding"
                        )
                    }
                };
            }
        });

        builder.build().execute(pool).await.map_err(|error| {
            SpiderError::engine(format!("failed to write sqlite store record: {error}"))
        })?;

        Ok(())
    }
}

impl Store for Sqlite {
    async fn open(&self, _spider_name: &str) -> Result<(), SpiderError> {
        self.pool().await?;
        Ok(())
    }

    async fn write(&self, item: &Item, spider_name: &str) -> Result<(), SpiderError> {
        let pool = self.pool().await?;
        self.insert_item(&pool, item, spider_name).await?;
        Ok(())
    }

    async fn batch_write(&self, items: &[Item], spider_name: &str) -> Result<(), SpiderError> {
        let pool = self.pool().await?;
        self.insert_items(&pool, items, spider_name).await?;
        Ok(())
    }

    async fn close(&self, _spider_name: &str) -> Result<(), SpiderError> {
        let pool = {
            let mut guard = self.pool.lock().await;
            guard.take()
        };

        if let Some(pool) = pool {
            pool.close().await;
        }

        Ok(())
    }
}

fn build_create_table_statement(table: &str, columns: &[FieldColumn]) -> String {
    let mut statement = format!(
        "CREATE TABLE IF NOT EXISTS {} (\"id\" INTEGER PRIMARY KEY AUTOINCREMENT, \"spider_name\" TEXT NOT NULL, \"item_json\" TEXT NOT NULL",
        quote_identifier(table)
    );

    for column in columns {
        statement.push_str(", ");
        statement.push_str(&quote_identifier(&column.name));
        statement.push(' ');
        statement.push_str(column.column_type.sqlite_type());
    }

    statement.push(')');
    statement
}

fn sqlite_connect_options(path: &Path) -> Result<SqliteConnectOptions, SpiderError> {
    SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map(|options| options.create_if_missing(true))
        .map_err(|error| SpiderError::engine(format!("invalid sqlite store path: {error}")))
}

fn normalize_sqlite_field_column_value(
    value: FieldColumnValue,
) -> Result<FieldColumnValue, SpiderError> {
    match value {
        FieldColumnValue::Json(value) => serde_json::to_string(&value)
            .map(FieldColumnValue::Text)
            .map_err(|error| {
                SpiderError::engine(format!(
                    "failed to serialize sqlite store json column: {error}"
                ))
            }),
        other => Ok(other),
    }
}

async fn ensure_parent_dir(path: &Path) -> Result<(), SpiderError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    tokio::fs::create_dir_all(parent).await.map_err(|error| {
        SpiderError::engine(format!("failed to create sqlite store directory: {error}"))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use sqlx::Row;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP_DB_ID: AtomicUsize = AtomicUsize::new(0);

    #[tokio::test]
    async fn sqlite_store_creates_table_and_stores_item_json_and_mapped_fields() {
        let path = unique_path("stores");
        let store = Sqlite::new(path.clone())
            .with_table("period_items")
            .with_field_column("title", "title", FieldColumnType::Text)
            .with_field_column("page", "page", FieldColumnType::Integer)
            .with_field_column("meta", "meta_json", FieldColumnType::Json);
        let item = Item::new()
            .with_field("title", Value::String("period".to_string()))
            .with_field("page", Value::Number(1.0))
            .with_field(
                "meta",
                Value::Object(
                    [("kind".to_string(), Value::String("front".to_string()))]
                        .into_iter()
                        .collect(),
                ),
            );

        store.open("news").await.unwrap();
        store.write(&item, "news").await.unwrap();
        store.close("news").await.unwrap();

        let pool = test_pool(&path).await;
        let row =
            sqlx::query("SELECT spider_name, item_json, title, page, meta_json FROM period_items")
                .fetch_one(&pool)
                .await
                .unwrap();

        let spider_name: String = row.get("spider_name");
        let item_json: String = row.get("item_json");
        let title: String = row.get("title");
        let page: i64 = row.get("page");
        let meta_json: String = row.get("meta_json");

        assert_eq!(spider_name, "news");
        assert_eq!(title, "period");
        assert_eq!(page, 1);
        assert_eq!(meta_json, "{\"kind\":\"front\"}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&item_json).unwrap(),
            serde_json::json!({
                "meta": {"kind": "front"},
                "page": 1.0,
                "title": "period"
            })
        );

        cleanup_path(&path).await;
    }

    #[tokio::test]
    async fn sqlite_store_inserts_null_for_missing_mapped_field() {
        let path = unique_path("nulls");
        let store = Sqlite::new(path.clone())
            .with_field_column("title", "title", FieldColumnType::Text)
            .with_field_column("summary", "summary", FieldColumnType::Text);
        let item = Item::new().with_field("title", Value::String("period".to_string()));

        store.open("news").await.unwrap();
        store.write(&item, "news").await.unwrap();

        let pool = test_pool(&path).await;
        let row = sqlx::query("SELECT title, summary FROM items")
            .fetch_one(&pool)
            .await
            .unwrap();

        let title: String = row.get("title");
        let summary: Option<String> = row.get("summary");

        assert_eq!(title, "period");
        assert_eq!(summary, None);

        cleanup_path(&path).await;
    }

    #[tokio::test]
    async fn sqlite_store_batch_write_inserts_multiple_rows() {
        let path = unique_path("batch");
        let store =
            Sqlite::new(path.clone()).with_field_column("title", "title", FieldColumnType::Text);
        let first = Item::new().with_field("title", Value::String("first".to_string()));
        let second = Item::new().with_field("title", Value::String("second".to_string()));

        store.open("news").await.unwrap();
        store.batch_write(&[first, second], "news").await.unwrap();

        let pool = test_pool(&path).await;
        let rows = sqlx::query("SELECT title FROM items ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
        let titles = rows
            .into_iter()
            .map(|row| row.get::<String, _>("title"))
            .collect::<Vec<_>>();

        assert_eq!(titles, vec!["first".to_string(), "second".to_string()]);

        cleanup_path(&path).await;
    }

    #[tokio::test]
    async fn sqlite_store_rejects_mapped_field_type_mismatch() {
        let path = unique_path("type_mismatch");
        let store =
            Sqlite::new(path.clone()).with_field_column("page", "page", FieldColumnType::Integer);
        let item = Item::new().with_field("page", Value::String("A01".to_string()));

        store.open("news").await.unwrap();
        let error = store.write(&item, "news").await.unwrap_err();

        assert_eq!(
            error,
            SpiderError::engine(
                "sqlite store field `page` cannot be stored in column `page` as integer: got text",
            )
        );

        cleanup_path(&path).await;
    }

    #[tokio::test]
    async fn sqlite_store_rejects_invalid_table_name() {
        let path = unique_path("invalid_table");
        let store = Sqlite::new(path.clone()).with_table("bad-name");

        let error = store.open("news").await.unwrap_err();

        assert_eq!(
            error,
            SpiderError::engine(
                "sqlite store table name must use only ASCII letters, digits, and underscores, and cannot start with a digit: bad-name",
            )
        );

        cleanup_path(&path).await;
    }

    async fn test_pool(path: &Path) -> SqlitePool {
        let options = sqlite_connect_options(path).unwrap();
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap()
    }

    fn unique_path(label: &str) -> PathBuf {
        let id = NEXT_TEMP_DB_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("halo_spider_store_sqlite_{label}_{id}.db"))
    }

    async fn cleanup_path(path: &Path) {
        let _ = tokio::fs::remove_file(path).await;
    }
}
