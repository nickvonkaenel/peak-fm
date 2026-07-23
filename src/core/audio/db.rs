use rusqlite::{params, Connection};
use std::path::Path;

pub struct Database {
    conn: Connection,
}

#[derive(Debug)]
pub struct AudioFileRecord {
    pub id: Option<i64>,
    pub file_path: String,
    pub file_name: String,
    pub description: Option<String>,
    pub sample_rate: Option<i32>,
    pub channels: Option<i32>,
}

impl Database {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("Failed to open database: {}", e))?;

        let db = Database { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                r#"
            CREATE TABLE IF NOT EXISTS audio_files (
                id INTEGER PRIMARY KEY,
                file_path TEXT UNIQUE NOT NULL,
                file_name TEXT NOT NULL,
                description TEXT,
                sample_rate INTEGER,
                channels INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_file_path ON audio_files(file_path);

            CREATE TABLE IF NOT EXISTS db_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
            )
            .map_err(|e| format!("Failed to initialize database schema: {}", e))?;

        Ok(())
    }

    pub fn set_pragmas(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA cache_size = -64000;
            PRAGMA temp_store = MEMORY;
            PRAGMA mmap_size = 268435456;
            "#,
            )
            .map_err(|e| format!("Failed to set database pragmas: {}", e))?;
        Ok(())
    }

    pub fn rebuild_database(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                r#"
            DROP TABLE IF EXISTS audio_files;
            DROP TABLE IF EXISTS db_metadata;
            "#,
            )
            .map_err(|e| format!("Failed to drop tables: {}", e))?;
        self.init_schema()?;
        Ok(())
    }

    pub fn get_file_by_path(&self, path: &str) -> Result<Option<AudioFileRecord>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_path, file_name, description, sample_rate, channels
             FROM audio_files WHERE file_path = ?1",
            )
            .map_err(|e| e.to_string())?;

        let result = stmt.query_row(params![path], |row| {
            Ok(AudioFileRecord {
                id: Some(row.get(0)?),
                file_path: row.get(1)?,
                file_name: row.get(2)?,
                description: row.get(3)?,
                sample_rate: row.get(4)?,
                channels: row.get(5)?,
            })
        });

        match result {
            Ok(file) => Ok(Some(file)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn insert_file(&self, file: &AudioFileRecord) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO audio_files (file_path, file_name, description, sample_rate, channels)
             VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    file.file_path,
                    file.file_name,
                    file.description,
                    file.sample_rate,
                    file.channels,
                ],
            )
            .map_err(|e| e.to_string())?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Insert multiple files in a single batch operation
    pub fn insert_files_batch(&self, files: &[AudioFileRecord]) -> Result<(), String> {
        if files.is_empty() {
            return Ok(());
        }

        // Build a single INSERT with multiple VALUES
        // SQLite has a limit on the number of parameters (usually 999)
        // With 5 params per file, we can do ~199 files per query
        const MAX_PARAMS: usize = 999;
        const PARAMS_PER_FILE: usize = 5;
        const MAX_FILES_PER_BATCH: usize = MAX_PARAMS / PARAMS_PER_FILE;

        for chunk in files.chunks(MAX_FILES_PER_BATCH) {
            let placeholders: Vec<String> = (0..chunk.len())
                .map(|i| {
                    let base = i * PARAMS_PER_FILE + 1;
                    format!(
                        "(?{}, ?{}, ?{}, ?{}, ?{})",
                        base,
                        base + 1,
                        base + 2,
                        base + 3,
                        base + 4
                    )
                })
                .collect();

            let sql = format!(
                "INSERT INTO audio_files (file_path, file_name, description, sample_rate, channels) VALUES {}",
                placeholders.join(", ")
            );

            let mut stmt = self.conn.prepare(&sql).map_err(|e| e.to_string())?;

            // Flatten all parameters
            let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            for file in chunk {
                params_vec.push(Box::new(file.file_path.clone()));
                params_vec.push(Box::new(file.file_name.clone()));
                params_vec.push(Box::new(file.description.clone()));
                params_vec.push(Box::new(file.sample_rate));
                params_vec.push(Box::new(file.channels));
            }

            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();

            stmt.execute(params_refs.as_slice())
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    pub fn begin_transaction(&self) -> Result<(), String> {
        self.conn
            .execute("BEGIN TRANSACTION", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn commit_transaction(&self) -> Result<(), String> {
        self.conn.execute("COMMIT", []).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn is_complete(&self) -> Result<bool, String> {
        let result: Result<String, _> = self.conn.query_row(
            "SELECT value FROM db_metadata WHERE key = 'complete'",
            [],
            |row| row.get(0),
        );

        match result {
            Ok(value) => Ok(value == "true"),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn mark_complete(&self) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();

        self.conn
            .execute(
                "INSERT OR REPLACE INTO db_metadata (key, value) VALUES ('complete', 'true')",
                [],
            )
            .map_err(|e| e.to_string())?;

        self.conn
            .execute(
                "INSERT OR REPLACE INTO db_metadata (key, value) VALUES ('completed_at', ?1)",
                params![now.to_string()],
            )
            .map_err(|e| e.to_string())?;

        Ok(())
    }

    pub fn mark_incomplete(&self) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM db_metadata WHERE key IN ('complete', 'completed_at')",
                [],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_all_files(&self) -> Result<Vec<AudioFileRecord>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_path, file_name, description, sample_rate, channels
             FROM audio_files",
            )
            .map_err(|e| e.to_string())?;

        let files = stmt
            .query_map([], |row| {
                Ok(AudioFileRecord {
                    id: Some(row.get(0)?),
                    file_path: row.get(1)?,
                    file_name: row.get(2)?,
                    description: row.get(3)?,
                    sample_rate: row.get(4)?,
                    channels: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;

        // Pre-allocate with reasonable starting capacity
        let mut result = Vec::with_capacity(10000);
        for file in files {
            result.push(file.map_err(|e| e.to_string())?);
        }

        Ok(result)
    }

    pub fn get_all_files_with_descriptions(
        &self,
    ) -> Result<Vec<(AudioFileRecord, Option<String>)>, String> {
        // Simple query from single table - description is already in the record
        let files = self.get_all_files()?;
        let result = files
            .into_iter()
            .map(|f| {
                let desc = f.description.clone();
                (f, desc)
            })
            .collect();
        Ok(result)
    }

    pub fn get_files_batch(&self, limit: i64, offset: i64) -> Result<Vec<AudioFileRecord>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, file_path, file_name, description, sample_rate, channels
             FROM audio_files
             ORDER BY file_path
             LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| e.to_string())?;

        let files = stmt
            .query_map(params![limit, offset], |row| {
                Ok(AudioFileRecord {
                    id: Some(row.get(0)?),
                    file_path: row.get(1)?,
                    file_name: row.get(2)?,
                    description: row.get(3)?,
                    sample_rate: row.get(4)?,
                    channels: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;

        // Pre-allocate with the limit size (max we'll get)
        let mut result = Vec::with_capacity(limit as usize);
        for file in files {
            result.push(file.map_err(|e| e.to_string())?);
        }

        Ok(result)
    }

    pub fn get_file_count(&self) -> Result<i64, String> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM audio_files", [], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        Ok(count)
    }
}
