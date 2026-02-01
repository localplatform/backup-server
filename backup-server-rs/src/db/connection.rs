use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;

pub type DbPool = Pool<SqliteConnectionManager>;

pub fn create_pool(db_path: &str) -> DbPool {
    let manager = SqliteConnectionManager::file(db_path);
    let pool = Pool::builder()
        .max_size(4)
        .build(manager)
        .expect("Failed to create DB pool");

    // Configure pragmas on a fresh connection
    let conn = pool.get().expect("Failed to get DB connection");
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;",
    )
    .expect("Failed to set PRAGMA");

    pool
}

pub fn close_pool(pool: &DbPool) {
    // r2d2 will close connections when the pool is dropped.
    // Attempt a checkpoint just in case (no-op in DELETE mode).
    if let Ok(conn) = pool.get() {
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(FULL)");
    }
}
