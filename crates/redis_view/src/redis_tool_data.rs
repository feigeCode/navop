//! Redis 工具标签页的数据加载与解析。

pub(crate) use crate::redis_tool_parsers::{
    build_publish_command, quote_command_arg, rows_from_pubsub_values,
    rows_from_slowlog_config_value, rows_from_slowlog_len_value, rows_from_slowlog_value,
    slowlog_reset_command,
};
use crate::{RedisConnection, RedisValue};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedisToolKind {
    Info,
    Memory,
    SlowLog,
    Monitor,
    PubSub,
    Chart,
}

#[derive(Clone, Debug)]
pub struct ToolRow {
    pub category: String,
    pub key: String,
    pub value: String,
}

impl RedisToolKind {
    pub fn tab_id(self) -> &'static str {
        match self {
            Self::Info => "redis-info",
            Self::Memory => "redis-memory",
            Self::SlowLog => "redis-slow-log",
            Self::Monitor => "redis-monitor",
            Self::PubSub => "redis-pub-sub",
            Self::Chart => "redis-chart",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Memory => "Memory",
            Self::SlowLog => "SlowLog",
            Self::Monitor => "Monitor",
            Self::PubSub => "Pub/Sub",
            Self::Chart => "Chart",
        }
    }

    pub fn section(self) -> Option<&'static str> {
        match self {
            Self::Info | Self::Chart => None,
            Self::Memory => Some("memory"),
            Self::Monitor => Some("commandstats"),
            Self::SlowLog | Self::PubSub => None,
        }
    }
}

pub async fn load_tool_rows(
    kind: RedisToolKind,
    db_index: u8,
    conn: &dyn RedisConnection,
) -> anyhow::Result<Vec<ToolRow>> {
    match kind {
        RedisToolKind::SlowLog => load_slowlog_rows(conn).await,
        RedisToolKind::Monitor => load_monitor_rows(conn).await,
        RedisToolKind::PubSub => load_pubsub_rows(conn).await,
        RedisToolKind::Chart => load_chart_rows(conn, db_index).await,
        _ => {
            let info = conn.info(kind.section()).await?;
            Ok(parse_info(&info))
        }
    }
}

async fn load_slowlog_rows(conn: &dyn RedisConnection) -> anyhow::Result<Vec<ToolRow>> {
    let mut rows = rows_from_slowlog_value(conn.execute_command("SLOWLOG GET 128").await?);
    if let Ok(value) = conn.execute_command("SLOWLOG LEN").await {
        rows.extend(rows_from_slowlog_len_value(value));
    }
    if let Ok(value) = conn
        .execute_command("CONFIG GET slowlog-log-slower-than slowlog-max-len")
        .await
    {
        rows.extend(rows_from_slowlog_config_value(value));
    }
    Ok(rows)
}

async fn load_monitor_rows(conn: &dyn RedisConnection) -> anyhow::Result<Vec<ToolRow>> {
    let mut rows = parse_info(&conn.info(Some("commandstats")).await?);
    if let Ok(stats) = conn.info(Some("stats")).await {
        rows.extend(parse_info(&stats));
    }
    Ok(rows)
}

async fn load_pubsub_rows(conn: &dyn RedisConnection) -> anyhow::Result<Vec<ToolRow>> {
    let channels = conn.execute_command("PUBSUB CHANNELS").await?;
    let patterns = conn.execute_command("PUBSUB NUMPAT").await?;
    let channel_names = string_list(&channels);
    let subscribers = if channel_names.is_empty() {
        RedisValue::Bulk(Vec::new())
    } else {
        let command = format!(
            "PUBSUB NUMSUB {}",
            channel_names
                .iter()
                .map(|channel| quote_command_arg(channel))
                .collect::<Vec<_>>()
                .join(" ")
        );
        conn.execute_command(&command).await?
    };
    let shard_channels = conn
        .execute_command("PUBSUB SHARDCHANNELS")
        .await
        .unwrap_or_else(|_| crate::RedisValue::Bulk(Vec::new()));
    let shard_names = string_list(&shard_channels);
    let shard_subscribers = if shard_names.is_empty() {
        crate::RedisValue::Bulk(Vec::new())
    } else {
        let command = format!(
            "PUBSUB SHARDNUMSUB {}",
            shard_names
                .iter()
                .map(|channel| quote_command_arg(channel))
                .collect::<Vec<_>>()
                .join(" ")
        );
        conn.execute_command(&command)
            .await
            .unwrap_or_else(|_| crate::RedisValue::Bulk(Vec::new()))
    };
    Ok(rows_from_pubsub_values(
        channels,
        subscribers,
        patterns,
        shard_channels,
        shard_subscribers,
    ))
}

async fn load_chart_rows(conn: &dyn RedisConnection, db_index: u8) -> anyhow::Result<Vec<ToolRow>> {
    let info = conn.info(None).await?;
    let mut rows = parse_info(&info)
        .into_iter()
        .filter(|row| {
            matches!(
                row.category.as_str(),
                "Memory" | "Stats" | "Clients" | "Keyspace"
            )
        })
        .collect::<Vec<_>>();
    let dbsize = conn.execute_command_in_db(db_index, "DBSIZE").await?;
    rows.push(ToolRow {
        category: "Keyspace".to_string(),
        key: format!("db{db_index}_keys"),
        value: dbsize.to_display_string(),
    });
    rows.push(ToolRow {
        category: "Keyspace".to_string(),
        key: "selected_db_keys".to_string(),
        value: dbsize.to_display_string(),
    });
    Ok(rows)
}

pub fn parse_info(info: &str) -> Vec<ToolRow> {
    let mut category = String::new();
    let mut rows = Vec::new();
    for line in info.lines() {
        if let Some(section) = line.strip_prefix("# ") {
            category = section.to_string();
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            rows.push(ToolRow {
                category: category.clone(),
                key: key.to_string(),
                value: value.to_string(),
            });
        }
    }
    rows
}

fn string_list(value: &RedisValue) -> Vec<String> {
    let crate::RedisValue::Bulk(items) = value else {
        return Vec::new();
    };
    items
        .iter()
        .map(crate::redis_tool_parsers::value_text)
        .collect()
}
