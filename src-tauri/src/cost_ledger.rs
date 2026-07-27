use rusqlite::{params, Connection};

pub const COST_FORMULA_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct ResolvedPrice {
    pub model_id: String,
    pub match_kind: &'static str,
    pub input_per_m: f64,
    pub output_per_m: f64,
    pub cache_read_per_m: f64,
    pub cache_creation_per_m: f64,
}

#[derive(Debug, Clone)]
pub struct CostRecord {
    pub cost: f64,
    pub provider_cost_amount: Option<f64>,
    pub cost_basis: &'static str,
    pub usage_completeness: &'static str,
    pub pricing_match: &'static str,
    pub pricing_model_id: Option<String>,
    pub input_price_per_m: Option<f64>,
    pub output_price_per_m: Option<f64>,
    pub cache_read_price_per_m: Option<f64>,
    pub cache_creation_price_per_m: Option<f64>,
    pub usage_fingerprint: String,
}

#[allow(clippy::too_many_arguments)]
pub fn build_cost_record(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    reasoning_tokens: u64,
    provider_cost_amount: Option<f64>,
    usage_complete: bool,
    resolved_price: Option<ResolvedPrice>,
) -> CostRecord {
    let usage_completeness = if usage_complete {
        "complete"
    } else {
        "partial"
    };
    let usage_fingerprint = serde_json::to_string(&(
        COST_FORMULA_VERSION,
        model,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        reasoning_tokens,
        provider_cost_amount,
    ))
    .expect("serializing a pricing fingerprint should not fail");

    if let Some(provider_cost) = provider_cost_amount {
        return CostRecord {
            cost: provider_cost,
            provider_cost_amount: Some(provider_cost),
            cost_basis: "provider",
            usage_completeness,
            pricing_match: "provider",
            pricing_model_id: None,
            input_price_per_m: None,
            output_price_per_m: None,
            cache_read_price_per_m: None,
            cache_creation_price_per_m: None,
            usage_fingerprint,
        };
    }

    if let Some(price) = resolved_price {
        let per_m = 1_000_000.0;
        let cost = (input_tokens as f64 / per_m) * price.input_per_m
            + (output_tokens as f64 / per_m) * price.output_per_m
            + (cache_read_tokens as f64 / per_m) * price.cache_read_per_m
            + (cache_creation_tokens as f64 / per_m) * price.cache_creation_per_m
            + (reasoning_tokens as f64 / per_m) * price.output_per_m;
        return CostRecord {
            cost,
            provider_cost_amount: None,
            cost_basis: "estimated",
            usage_completeness,
            pricing_match: price.match_kind,
            pricing_model_id: Some(price.model_id),
            input_price_per_m: Some(price.input_per_m),
            output_price_per_m: Some(price.output_per_m),
            cache_read_price_per_m: Some(price.cache_read_per_m),
            cache_creation_price_per_m: Some(price.cache_creation_per_m),
            usage_fingerprint,
        };
    }

    CostRecord {
        cost: 0.0,
        provider_cost_amount: None,
        cost_basis: "unpriced",
        usage_completeness,
        pricing_match: "unmatched",
        pricing_model_id: None,
        input_price_per_m: None,
        output_price_per_m: None,
        cache_read_price_per_m: None,
        cache_creation_price_per_m: None,
        usage_fingerprint,
    }
}

pub fn ensure_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_costs (
            session_id TEXT NOT NULL,
            source TEXT NOT NULL,
            date TEXT NOT NULL,
            cost REAL NOT NULL DEFAULT 0,
            provider_cost_amount REAL,
            cost_basis TEXT NOT NULL,
            usage_completeness TEXT NOT NULL,
            pricing_match TEXT NOT NULL,
            pricing_model_id TEXT,
            input_price_per_m REAL,
            output_price_per_m REAL,
            cache_read_price_per_m REAL,
            cache_creation_price_per_m REAL,
            cost_formula_version INTEGER NOT NULL,
            usage_fingerprint TEXT NOT NULL,
            priced_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (session_id, source, date)
        );
        CREATE INDEX IF NOT EXISTS idx_session_costs_basis ON session_costs(cost_basis);",
    )
    .map_err(|e| e.to_string())?;

    // Existing v0.2 rows do not record whether a zero OpenCode cost came from the
    // provider or from a failed estimate. Preserve positive OpenCode costs as legacy
    // provider values; keep every ambiguous zero explicitly unpriced until a rescan.
    conn.execute(
        "INSERT OR IGNORE INTO session_costs (
            session_id, source, date, cost, provider_cost_amount, cost_basis,
            usage_completeness, pricing_match, pricing_model_id,
            cost_formula_version, usage_fingerprint, priced_at
         )
         SELECT session_id, source, date, cost,
                CASE WHEN source = 'opencode' AND cost > 0 THEN cost ELSE NULL END,
                CASE
                  WHEN source = 'opencode' AND cost > 0 THEN 'provider'
                  WHEN cost > 0 THEN 'estimated'
                  ELSE 'unpriced'
                END,
                'partial',
                'legacy',
                NULL,
                ?1,
                printf('legacy|%s|%s|%s|%d|%d|%d|%d|%d|%d|%.17g',
                    session_id, source, model, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens,
                    reasoning_output_tokens, ?1, cost),
                COALESCE(scanned_at, datetime('now'))
         FROM sessions",
        params![COST_FORMULA_VERSION],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

pub fn upsert(
    conn: &Connection,
    session_id: &str,
    source: &str,
    date: &str,
    record: &CostRecord,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO session_costs (
            session_id, source, date, cost, provider_cost_amount, cost_basis,
            usage_completeness, pricing_match, pricing_model_id,
            input_price_per_m, output_price_per_m, cache_read_price_per_m,
            cache_creation_price_per_m, cost_formula_version, usage_fingerprint, priced_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
            datetime('now')
         )
         ON CONFLICT(session_id, source, date) DO UPDATE SET
            cost = excluded.cost,
            provider_cost_amount = excluded.provider_cost_amount,
            cost_basis = excluded.cost_basis,
            usage_completeness = excluded.usage_completeness,
            pricing_match = excluded.pricing_match,
            pricing_model_id = excluded.pricing_model_id,
            input_price_per_m = excluded.input_price_per_m,
            output_price_per_m = excluded.output_price_per_m,
            cache_read_price_per_m = excluded.cache_read_price_per_m,
            cache_creation_price_per_m = excluded.cache_creation_price_per_m,
            cost_formula_version = excluded.cost_formula_version,
            usage_fingerprint = excluded.usage_fingerprint,
            priced_at = datetime('now')
         WHERE session_costs.usage_fingerprint <> excluded.usage_fingerprint",
        params![
            session_id,
            source,
            date,
            record.cost,
            record.provider_cost_amount,
            record.cost_basis,
            record.usage_completeness,
            record.pricing_match,
            record.pricing_model_id,
            record.input_price_per_m,
            record.output_price_per_m,
            record.cache_read_price_per_m,
            record.cache_creation_price_per_m,
            COST_FORMULA_VERSION,
            record.usage_fingerprint,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                session_id TEXT NOT NULL, source TEXT NOT NULL, date TEXT NOT NULL,
                model TEXT, input_tokens INTEGER, output_tokens INTEGER,
                cache_read_tokens INTEGER, cache_creation_tokens INTEGER,
                reasoning_output_tokens INTEGER, cost REAL, scanned_at TEXT,
                PRIMARY KEY (session_id, source, date)
            );",
        )
        .unwrap();
        ensure_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn provider_zero_is_actual_when_presence_is_explicit() {
        let record = build_cost_record("free-model", 100, 20, 0, 0, 0, Some(0.0), true, None);
        assert_eq!(record.cost_basis, "provider");
        assert_eq!(record.cost, 0.0);
        assert_eq!(record.provider_cost_amount, Some(0.0));
    }

    #[test]
    fn estimated_cost_uses_the_saved_price_components() {
        let record = build_cost_record(
            "model",
            1_000_000,
            1_000_000,
            1_000_000,
            1_000_000,
            1_000_000,
            None,
            true,
            Some(ResolvedPrice {
                model_id: "provider/model".to_string(),
                match_kind: "exact",
                input_per_m: 2.0,
                output_per_m: 4.0,
                cache_read_per_m: 0.2,
                cache_creation_per_m: 2.5,
            }),
        );
        assert_eq!(record.cost_basis, "estimated");
        assert!((record.cost - 12.7).abs() < f64::EPSILON);
    }

    #[test]
    fn unchanged_fingerprint_preserves_the_original_snapshot_time() {
        let conn = test_connection();
        let record = build_cost_record(
            "model",
            100,
            20,
            0,
            0,
            0,
            None,
            true,
            Some(ResolvedPrice {
                model_id: "provider/model".to_string(),
                match_kind: "exact",
                input_per_m: 2.0,
                output_per_m: 4.0,
                cache_read_per_m: 0.2,
                cache_creation_per_m: 2.5,
            }),
        );
        upsert(&conn, "s1", "codex", "2026-07-27", &record).unwrap();
        conn.execute(
            "UPDATE session_costs SET priced_at = '2026-01-01 00:00:00'
             WHERE session_id = 's1'",
            [],
        )
        .unwrap();
        upsert(&conn, "s1", "codex", "2026-07-27", &record).unwrap();
        let priced_at: String = conn
            .query_row(
                "SELECT priced_at FROM session_costs WHERE session_id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(priced_at, "2026-01-01 00:00:00");
    }
}
