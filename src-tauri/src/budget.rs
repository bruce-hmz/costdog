use chrono::{Datelike, Local, NaiveDate};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

const SETTING_KEY: &str = "budget.monthly";
const MAX_BUDGET_USD: f64 = 1_000_000.0;

#[derive(Debug, Serialize, Deserialize)]
struct BudgetConfig {
    amount_usd: Option<f64>,
    revision: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatus {
    pub monthly_limit: Option<f64>,
    pub revision: u64,
    pub priced_month_to_date: f64,
    pub remaining: Option<f64>,
    pub percent_used: Option<f64>,
    pub unpriced_token_share: f64,
    pub forecast: Option<f64>,
    pub confidence: &'static str,
    pub reason_unavailable: Option<String>,
    pub data_days: u64,
    pub priced_sessions: u64,
    pub period: String,
}

pub struct BudgetAlertEvent {
    pub key: String,
    pub period: String,
    pub level: String,
    pub message: String,
}

pub fn set_budget(conn: &Connection, amount_usd: Option<f64>) -> Result<(), String> {
    if let Some(amount) = amount_usd {
        if !amount.is_finite() || amount <= 0.0 || amount > MAX_BUDGET_USD {
            return Err(format!(
                "Monthly budget must be a finite USD amount greater than 0 and at most {MAX_BUDGET_USD}"
            ));
        }
    }
    let revision = load_config(conn)?.map_or(1, |config| config.revision + 1);
    let value = serde_json::to_string(&BudgetConfig {
        amount_usd,
        revision,
    })
    .map_err(|error| error.to_string())?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    tx.execute(
        "INSERT INTO app_settings(key, value, updated_at)
         VALUES (?1, ?2, datetime('now','localtime'))
         ON CONFLICT(key) DO UPDATE SET
           value=excluded.value,
           updated_at=excluded.updated_at",
        params![SETTING_KEY, value],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "UPDATE alerts SET dismissed=1
         WHERE alert_key LIKE 'budget:%' AND dismissed=0",
        [],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())
}

pub fn get_status(
    conn: &Connection,
    pricing_cache_age_hours: Option<i64>,
) -> Result<BudgetStatus, String> {
    let now = Local::now().date_naive();
    let month_start = NaiveDate::from_ymd_opt(now.year(), now.month(), 1)
        .expect("the first day of a valid month exists");
    let next_month_start = if now.month() == 12 {
        NaiveDate::from_ymd_opt(now.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(now.year(), now.month() + 1, 1)
    }
    .expect("the first day of the next month exists");
    let days_in_month = (next_month_start - month_start).num_days() as f64;
    let elapsed_days = (now - month_start).num_days() as f64 + 1.0;
    let start = month_start.format("%Y-%m-%d").to_string();
    let end = now.format("%Y-%m-%d").to_string();

    let (priced_cost, estimated_cost, total_tokens, unpriced_tokens, data_days, priced_sessions): (
        f64,
        f64,
        u64,
        u64,
        u64,
        u64,
    ) = conn
        .query_row(
            "SELECT
               COALESCE(SUM(CASE WHEN sc.cost_basis IN ('provider','estimated')
                    THEN sc.cost ELSE 0 END),0),
               COALESCE(SUM(CASE WHEN sc.cost_basis='estimated' THEN sc.cost ELSE 0 END),0),
               COALESCE(SUM(s.input_tokens+s.output_tokens+s.cache_read_tokens+
                            s.cache_creation_tokens+s.reasoning_output_tokens),0),
               COALESCE(SUM(CASE WHEN sc.cost_basis='unpriced'
                    THEN s.input_tokens+s.output_tokens+s.cache_read_tokens+
                         s.cache_creation_tokens+s.reasoning_output_tokens ELSE 0 END),0),
               COUNT(DISTINCT CASE WHEN sc.cost_basis IN ('provider','estimated')
                    THEN s.date END),
               COUNT(DISTINCT CASE WHEN sc.cost_basis IN ('provider','estimated')
                    THEN s.source || char(0) || s.session_id END)
             FROM sessions s
             JOIN session_costs sc
               ON sc.session_id=s.session_id AND sc.source=s.source AND sc.date=s.date
             WHERE s.date >= ?1 AND s.date <= ?2",
            params![start, end],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;

    let unpriced_share = if total_tokens == 0 {
        0.0
    } else {
        unpriced_tokens as f64 / total_tokens as f64
    };
    let price_is_high_fresh =
        estimated_cost == 0.0 || pricing_cache_age_hours.is_some_and(|hours| hours <= 48);
    let price_is_medium_fresh =
        estimated_cost == 0.0 || pricing_cache_age_hours.is_some_and(|hours| hours <= 24 * 7);
    let (confidence, reason_unavailable) = if data_days >= 7
        && priced_sessions >= 5
        && unpriced_share <= 0.05
        && price_is_high_fresh
    {
        ("high", None)
    } else if data_days >= 3
        && priced_sessions >= 5
        && unpriced_share <= 0.20
        && price_is_medium_fresh
    {
        ("medium", None)
    } else {
        let reason = if unpriced_share > 0.20 {
            format!(
                "{:.0}% of token usage is unpriced; at most 20% is required",
                unpriced_share * 100.0
            )
        } else if priced_sessions < 5 {
            format!("At least 5 priced sessions are required; found {priced_sessions}")
        } else if data_days < 3 {
            format!("At least 3 data days are required; found {data_days}")
        } else {
            "Pricing data is too old or unavailable".to_string()
        };
        ("unavailable", Some(reason))
    };
    let forecast = if confidence == "unavailable" {
        None
    } else {
        Some(priced_cost / elapsed_days * days_in_month)
    };
    let config = load_config(conn)?.unwrap_or(BudgetConfig {
        amount_usd: None,
        revision: 0,
    });
    let remaining = config
        .amount_usd
        .map(|limit| (limit - priced_cost).max(0.0));
    let percent_used = config.amount_usd.map(|limit| priced_cost / limit * 100.0);

    Ok(BudgetStatus {
        monthly_limit: config.amount_usd,
        revision: config.revision,
        priced_month_to_date: priced_cost,
        remaining,
        percent_used,
        unpriced_token_share: unpriced_share,
        forecast,
        confidence,
        reason_unavailable,
        data_days,
        priced_sessions,
        period: now.format("%Y-%m").to_string(),
    })
}

pub fn threshold_events(status: &BudgetStatus) -> Vec<BudgetAlertEvent> {
    let Some(limit) = status.monthly_limit else {
        return Vec::new();
    };
    let mut events = Vec::new();
    let used = status.percent_used.unwrap_or(0.0);
    for threshold in [70_u64, 90, 100] {
        if used >= threshold as f64 {
            events.push(BudgetAlertEvent {
                key: format!("budget:{}:{threshold}", status.revision),
                period: status.period.clone(),
                level: if threshold >= 100 {
                    "danger".to_string()
                } else {
                    "warn".to_string()
                },
                message: format!(
                    "Monthly budget is at {:.0}%: ${:.2} of ${limit:.2}",
                    used, status.priced_month_to_date
                ),
            });
        }
    }
    if status.forecast.is_some_and(|forecast| forecast > limit) {
        events.push(BudgetAlertEvent {
            key: format!("budget:{}:forecast", status.revision),
            period: status.period.clone(),
            level: "warn".to_string(),
            message: format!(
                "Forecast exceeds the monthly budget: ${:.2} projected vs ${limit:.2}",
                status.forecast.unwrap_or_default()
            ),
        });
    }
    events
}

fn load_config(conn: &Connection) -> Result<Option<BudgetConfig>, String> {
    let value = conn.query_row(
        "SELECT value FROM app_settings WHERE key=?1",
        params![SETTING_KEY],
        |row| row.get::<_, String>(0),
    );
    match value {
        Ok(value) => serde_json::from_str(&value)
            .map(Some)
            .map_err(|error| format!("Invalid stored monthly budget: {error}")),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE app_settings (
                key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT
            );
            CREATE TABLE alerts (
                id INTEGER PRIMARY KEY, alert_key TEXT, dismissed INTEGER DEFAULT 0
            );
            CREATE TABLE sessions (
                session_id TEXT, source TEXT, date TEXT,
                input_tokens INTEGER, output_tokens INTEGER,
                cache_read_tokens INTEGER, cache_creation_tokens INTEGER,
                reasoning_output_tokens INTEGER
            );
            CREATE TABLE session_costs (
                session_id TEXT, source TEXT, date TEXT, cost REAL,
                cost_basis TEXT, usage_completeness TEXT
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn rejects_invalid_amounts_and_increments_revision() {
        let conn = connection();
        for invalid in [0.0, -1.0, f64::INFINITY, 1_000_001.0] {
            assert!(set_budget(&conn, Some(invalid)).is_err());
        }
        set_budget(&conn, Some(100.0)).unwrap();
        set_budget(&conn, Some(120.0)).unwrap();
        assert_eq!(load_config(&conn).unwrap().unwrap().revision, 2);
    }

    #[test]
    fn thresholds_are_revision_scoped_and_include_forecast() {
        let status = BudgetStatus {
            monthly_limit: Some(100.0),
            revision: 4,
            priced_month_to_date: 105.0,
            remaining: Some(0.0),
            percent_used: Some(105.0),
            unpriced_token_share: 0.0,
            forecast: Some(150.0),
            confidence: "high",
            reason_unavailable: None,
            data_days: 10,
            priced_sessions: 12,
            period: "2026-07".to_string(),
        };
        let events = threshold_events(&status);
        assert_eq!(events.len(), 4);
        assert!(events
            .iter()
            .all(|event| event.key.starts_with("budget:4:")));
    }
}
