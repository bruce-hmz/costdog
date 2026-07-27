use chrono::{Datelike, Duration, Local, NaiveDate};
use rusqlite::{params_from_iter, types::Value, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

const SOURCES: &[&str] = &["claude-code", "codex", "zcode", "opencode"];
const ACTIVITIES: &[&str] = &[
    "feature", "bugfix", "refactor", "docs", "research", "debug", "agent", "explore", "other",
];

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsFilters {
    pub source: Option<String>,
    pub model: Option<String>,
    pub project_key: Option<String>,
    pub activity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsTotals {
    pub sessions: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub cost: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyPoint {
    pub date: String,
    #[serde(flatten)]
    pub totals: AnalyticsTotals,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakdownItem {
    pub key: String,
    pub display_name: String,
    pub sessions: u64,
    pub tokens: u64,
    pub cost: f64,
}

#[derive(Debug, Serialize)]
pub struct Breakdowns {
    pub source: Vec<BreakdownItem>,
    pub model: Vec<BreakdownItem>,
    pub project: Vec<BreakdownItem>,
    pub activity: Vec<BreakdownItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contribution {
    pub dimension: String,
    pub key: String,
    pub display_name: String,
    pub current_cost: f64,
    pub previous_cost: f64,
    pub delta: f64,
    pub share_of_net_change: f64,
    pub direction: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsQuality {
    pub provider_cost: f64,
    pub estimated_cost: f64,
    pub unpriced_sessions: u64,
    pub unpriced_tokens: u64,
    pub partial_sessions: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsResponse {
    pub range: String,
    pub comparison_range: String,
    pub totals: AnalyticsTotals,
    pub comparison_totals: AnalyticsTotals,
    pub daily_series: Vec<DailyPoint>,
    pub breakdowns: Breakdowns,
    pub contributions: Vec<Contribution>,
    pub data_quality: AnalyticsQuality,
}

#[derive(Clone, Copy)]
enum Dimension {
    Source,
    Model,
    Project,
    Activity,
}

impl Dimension {
    fn name(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Model => "model",
            Self::Project => "project",
            Self::Activity => "activity",
        }
    }

    fn key_sql(self) -> &'static str {
        match self {
            Self::Source => "s.source",
            Self::Model => "COALESCE(NULLIF(s.model,''), 'unknown')",
            Self::Project => "COALESCE(NULLIF(s.project_key,''), 'unknown:' || s.source)",
            Self::Activity => {
                "COALESCE(NULLIF(s.activity_category_override,''), NULLIF(s.activity_category,''), 'other')"
            }
        }
    }

    fn display_sql(self) -> &'static str {
        match self {
            Self::Project => "COALESCE(NULLIF(s.project_display,''), 'Unknown project')",
            _ => self.key_sql(),
        }
    }
}

struct Window {
    start: NaiveDate,
    end: NaiveDate,
    previous_start: NaiveDate,
    previous_end: NaiveDate,
}

pub fn get_analytics(
    conn: &Connection,
    range: &str,
    filters: AnalyticsFilters,
) -> Result<AnalyticsResponse, String> {
    validate_filters(&filters)?;
    let window = resolve_window(range)?;
    let start = window.start.format("%Y-%m-%d").to_string();
    let end = window.end.format("%Y-%m-%d").to_string();
    let previous_start = window.previous_start.format("%Y-%m-%d").to_string();
    let previous_end = window.previous_end.format("%Y-%m-%d").to_string();

    let totals = query_totals(conn, &start, &end, &filters)?;
    let comparison_totals = query_totals(conn, &previous_start, &previous_end, &filters)?;
    let daily_series = query_daily_series(conn, &window, &filters)?;
    let dimensions = [
        Dimension::Source,
        Dimension::Model,
        Dimension::Project,
        Dimension::Activity,
    ];
    let mut current_breakdowns = Vec::new();
    let mut contributions = Vec::new();
    for dimension in dimensions {
        let current = query_breakdown(conn, &start, &end, &filters, dimension)?;
        let previous = query_breakdown(conn, &previous_start, &previous_end, &filters, dimension)?;
        contributions.extend(contribution_items(
            dimension,
            &current,
            &previous,
            totals.cost - comparison_totals.cost,
        ));
        current_breakdowns.push(current);
    }
    contributions.sort_by(|left, right| {
        right
            .delta
            .abs()
            .total_cmp(&left.delta.abs())
            .then_with(|| left.dimension.cmp(&right.dimension))
            .then_with(|| left.key.cmp(&right.key))
    });

    Ok(AnalyticsResponse {
        range: format!("{start}..{end}"),
        comparison_range: format!("{previous_start}..{previous_end}"),
        totals,
        comparison_totals,
        daily_series,
        breakdowns: Breakdowns {
            source: current_breakdowns.remove(0),
            model: current_breakdowns.remove(0),
            project: current_breakdowns.remove(0),
            activity: current_breakdowns.remove(0),
        },
        contributions,
        data_quality: query_quality(conn, &start, &end, &filters)?,
    })
}

fn validate_filters(filters: &AnalyticsFilters) -> Result<(), String> {
    if let Some(source) = &filters.source {
        if !SOURCES.contains(&source.as_str()) {
            return Err(format!("Invalid source filter: {source}"));
        }
    }
    if let Some(activity) = &filters.activity {
        if !ACTIVITIES.contains(&activity.as_str()) {
            return Err(format!("Invalid activity filter: {activity}"));
        }
    }
    Ok(())
}

fn resolve_window(range: &str) -> Result<Window, String> {
    let end = Local::now().date_naive();
    let start = match range {
        "today" => end,
        "7d" => end - Duration::days(6),
        "30d" => end - Duration::days(29),
        "month" => NaiveDate::from_ymd_opt(end.year(), end.month(), 1)
            .expect("the first day of a valid month exists"),
        _ => return Err(format!("Invalid analytics range: {range}")),
    };
    let length = (end - start).num_days() + 1;
    let previous_end = start - Duration::days(1);
    let previous_start = previous_end - Duration::days(length - 1);
    Ok(Window {
        start,
        end,
        previous_start,
        previous_end,
    })
}

fn filtered_where(start: &str, end: &str, filters: &AnalyticsFilters) -> (String, Vec<Value>) {
    let mut sql = "s.date >= ? AND s.date <= ?".to_string();
    let mut values = vec![Value::Text(start.to_string()), Value::Text(end.to_string())];
    for (column, value) in [
        ("s.source", filters.source.as_ref()),
        ("s.model", filters.model.as_ref()),
        ("s.project_key", filters.project_key.as_ref()),
    ] {
        if let Some(value) = value {
            sql.push_str(&format!(" AND {column} = ?"));
            values.push(Value::Text(value.clone()));
        }
    }
    if let Some(activity) = &filters.activity {
        sql.push_str(
            " AND COALESCE(NULLIF(s.activity_category_override,''), \
             NULLIF(s.activity_category,''), 'other') = ?",
        );
        values.push(Value::Text(activity.clone()));
    }
    (sql, values)
}

fn query_totals(
    conn: &Connection,
    start: &str,
    end: &str,
    filters: &AnalyticsFilters,
) -> Result<AnalyticsTotals, String> {
    let (where_sql, values) = filtered_where(start, end, filters);
    let sql = format!(
        "SELECT
           COUNT(DISTINCT s.source || char(0) || s.session_id),
           COALESCE(SUM(s.input_tokens),0),
           COALESCE(SUM(s.output_tokens),0),
           COALESCE(SUM(s.cache_read_tokens),0),
           COALESCE(SUM(s.cache_creation_tokens),0),
           COALESCE(SUM(s.reasoning_output_tokens),0),
           COALESCE(SUM(sc.cost),0)
         FROM sessions s
         JOIN session_costs sc
           ON sc.session_id=s.session_id AND sc.source=s.source AND sc.date=s.date
         WHERE {where_sql}"
    );
    conn.query_row(&sql, params_from_iter(values.iter()), |row| {
        Ok(AnalyticsTotals {
            sessions: row.get(0)?,
            input_tokens: row.get(1)?,
            output_tokens: row.get(2)?,
            cache_read_tokens: row.get(3)?,
            cache_creation_tokens: row.get(4)?,
            reasoning_output_tokens: row.get(5)?,
            cost: row.get(6)?,
        })
    })
    .map_err(|error| error.to_string())
}

fn query_daily_series(
    conn: &Connection,
    window: &Window,
    filters: &AnalyticsFilters,
) -> Result<Vec<DailyPoint>, String> {
    let start = window.start.format("%Y-%m-%d").to_string();
    let end = window.end.format("%Y-%m-%d").to_string();
    let (where_sql, values) = filtered_where(&start, &end, filters);
    let sql = format!(
        "SELECT s.date,
           COUNT(DISTINCT s.source || char(0) || s.session_id),
           COALESCE(SUM(s.input_tokens),0), COALESCE(SUM(s.output_tokens),0),
           COALESCE(SUM(s.cache_read_tokens),0), COALESCE(SUM(s.cache_creation_tokens),0),
           COALESCE(SUM(s.reasoning_output_tokens),0), COALESCE(SUM(sc.cost),0)
         FROM sessions s
         JOIN session_costs sc
           ON sc.session_id=s.session_id AND sc.source=s.source AND sc.date=s.date
         WHERE {where_sql}
         GROUP BY s.date ORDER BY s.date"
    );
    let mut stmt = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(values.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                AnalyticsTotals {
                    sessions: row.get(1)?,
                    input_tokens: row.get(2)?,
                    output_tokens: row.get(3)?,
                    cache_read_tokens: row.get(4)?,
                    cache_creation_tokens: row.get(5)?,
                    reasoning_output_tokens: row.get(6)?,
                    cost: row.get(7)?,
                },
            ))
        })
        .map_err(|error| error.to_string())?;
    let by_date = rows
        .filter_map(Result::ok)
        .collect::<BTreeMap<String, AnalyticsTotals>>();

    let mut result = Vec::new();
    let mut date = window.start;
    while date <= window.end {
        let key = date.format("%Y-%m-%d").to_string();
        result.push(DailyPoint {
            date: key.clone(),
            totals: by_date.get(&key).cloned().unwrap_or_default(),
        });
        date += Duration::days(1);
    }
    Ok(result)
}

fn query_breakdown(
    conn: &Connection,
    start: &str,
    end: &str,
    filters: &AnalyticsFilters,
    dimension: Dimension,
) -> Result<Vec<BreakdownItem>, String> {
    let (where_sql, values) = filtered_where(start, end, filters);
    let key = dimension.key_sql();
    let display = dimension.display_sql();
    let sql = format!(
        "SELECT {key}, {display},
           COUNT(DISTINCT s.source || char(0) || s.session_id),
           COALESCE(SUM(s.input_tokens+s.output_tokens+s.cache_read_tokens+
                        s.cache_creation_tokens+s.reasoning_output_tokens),0),
           COALESCE(SUM(sc.cost),0)
         FROM sessions s
         JOIN session_costs sc
           ON sc.session_id=s.session_id AND sc.source=s.source AND sc.date=s.date
         WHERE {where_sql}
         GROUP BY {key}, {display}
         ORDER BY SUM(sc.cost) DESC, {key}"
    );
    let mut stmt = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(values.iter()), |row| {
            Ok(BreakdownItem {
                key: row.get(0)?,
                display_name: row.get(1)?,
                sessions: row.get(2)?,
                tokens: row.get(3)?,
                cost: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn contribution_items(
    dimension: Dimension,
    current: &[BreakdownItem],
    previous: &[BreakdownItem],
    net_change: f64,
) -> Vec<Contribution> {
    let mut merged: HashMap<String, (String, f64, f64)> = HashMap::new();
    for item in current {
        merged.insert(
            item.key.clone(),
            (item.display_name.clone(), item.cost, 0.0),
        );
    }
    for item in previous {
        let entry = merged
            .entry(item.key.clone())
            .or_insert((item.display_name.clone(), 0.0, 0.0));
        entry.2 = item.cost;
    }
    merged
        .into_iter()
        .map(|(key, (display_name, current_cost, previous_cost))| {
            let delta = current_cost - previous_cost;
            Contribution {
                dimension: dimension.name().to_string(),
                key,
                display_name,
                current_cost,
                previous_cost,
                delta,
                share_of_net_change: if net_change.abs() < f64::EPSILON {
                    0.0
                } else {
                    delta / net_change
                },
                direction: if delta > 0.0 {
                    "up"
                } else if delta < 0.0 {
                    "down"
                } else {
                    "flat"
                },
            }
        })
        .collect()
}

fn query_quality(
    conn: &Connection,
    start: &str,
    end: &str,
    filters: &AnalyticsFilters,
) -> Result<AnalyticsQuality, String> {
    let (where_sql, values) = filtered_where(start, end, filters);
    let sql = format!(
        "SELECT
           COALESCE(SUM(CASE WHEN sc.cost_basis='provider' THEN sc.cost ELSE 0 END),0),
           COALESCE(SUM(CASE WHEN sc.cost_basis='estimated' THEN sc.cost ELSE 0 END),0),
           COUNT(DISTINCT CASE WHEN sc.cost_basis='unpriced'
             THEN s.source || char(0) || s.session_id END),
           COALESCE(SUM(CASE WHEN sc.cost_basis='unpriced'
             THEN s.input_tokens+s.output_tokens+s.cache_read_tokens+
                  s.cache_creation_tokens+s.reasoning_output_tokens ELSE 0 END),0),
           COUNT(DISTINCT CASE WHEN sc.usage_completeness='partial'
             THEN s.source || char(0) || s.session_id END)
         FROM sessions s
         JOIN session_costs sc
           ON sc.session_id=s.session_id AND sc.source=s.source AND sc.date=s.date
         WHERE {where_sql}"
    );
    conn.query_row(&sql, params_from_iter(values.iter()), |row| {
        Ok(AnalyticsQuality {
            provider_cost: row.get(0)?,
            estimated_cost: row.get(1)?,
            unpriced_sessions: row.get(2)?,
            unpriced_tokens: row.get(3)?,
            partial_sessions: row.get(4)?,
        })
    })
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_enum_filters_fail_fast() {
        let error = validate_filters(&AnalyticsFilters {
            source: Some("cursor".to_string()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(error.contains("cursor"));
    }

    #[test]
    fn seven_day_window_is_inclusive_and_has_equal_comparison() {
        let window = resolve_window("7d").unwrap();
        assert_eq!((window.end - window.start).num_days(), 6);
        assert_eq!((window.previous_end - window.previous_start).num_days(), 6);
        assert_eq!((window.start - window.previous_end).num_days(), 1);
    }

    #[test]
    fn contributions_keep_positive_and_negative_changes() {
        let current = vec![BreakdownItem {
            key: "a".to_string(),
            display_name: "A".to_string(),
            sessions: 1,
            tokens: 1,
            cost: 7.0,
        }];
        let previous = vec![
            BreakdownItem {
                key: "a".to_string(),
                display_name: "A".to_string(),
                sessions: 1,
                tokens: 1,
                cost: 2.0,
            },
            BreakdownItem {
                key: "b".to_string(),
                display_name: "B".to_string(),
                sessions: 1,
                tokens: 1,
                cost: 3.0,
            },
        ];
        let items = contribution_items(Dimension::Project, &current, &previous, 2.0);
        assert!(items
            .iter()
            .any(|item| item.key == "a" && item.delta == 5.0));
        assert!(items
            .iter()
            .any(|item| item.key == "b" && item.delta == -3.0));
        assert!((items.iter().map(|item| item.delta).sum::<f64>() - 2.0).abs() < 0.01);
    }
}
