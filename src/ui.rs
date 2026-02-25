use slint::{PlatformError, Weak, Window};
use sqlx::{pool, SqlitePool};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

slint::include_modules!();

pub(crate) fn run_ui(pool: sqlx::SqlitePool) -> Result<(), PlatformError> {
    let main_window = MainWindow::new()?;
    let window_weak = main_window.as_weak();

    tokio::spawn(async move {
        refresh_ui(pool, window_weak).await;
    });

    tracing::info!("starting ui");
    main_window.run()
}

async fn refresh_ui(pool: sqlx::SqlitePool, window_weak: Weak<MainWindow>) {
    let mut interval = time::interval(std::time::Duration::from_secs(10));

    loop {
        let count_1m = get_key_count(&pool, 60).await;
        let count_5m = get_key_count(&pool, 300).await;
        let count_15m = get_key_count(&pool, 900).await;
        let avg_1m = get_wpm(&pool, 60).await;
        let avg_5m = get_wpm(&pool, 300).await;
        let avg_15m = get_wpm(&pool, 900).await;
        let data_1m = get_keycount_details(&pool, 60, 5).await;
        let data_5m = get_keycount_details(&pool, 300, 25).await;
        let data_15m = get_keycount_details(&pool, 900, 75).await;

        tracing::info!("key count: {} / {} / {}", count_1m, count_5m, count_15m);

        window_weak.upgrade_in_event_loop(move |window| {
            window.set_key_count_1m(count_1m);
            window.set_key_count_5m(count_5m);
            window.set_key_count_15m(count_15m);
            window.set_wpm_1m(avg_1m);
            window.set_wpm_5m(avg_5m);
            window.set_wpm_15m(avg_15m);

            let is_compact = window.get_is_compact();
            let (w, h) = if is_compact {
                (150.0, 20.0)
            } else {
                (400.0, 80.0)
            };

            window.set_chart_path_1m(
                generate_path(data_1m, window.get_chart_w_1m(), window.get_chart_h_1m()).into(),
            );
            window.set_chart_path_5m(
                generate_path(data_5m, window.get_chart_w_5m(), window.get_chart_h_5m()).into(),
            );
            window.set_chart_path_15m(
                generate_path(data_15m, window.get_chart_w_15m(), window.get_chart_h_15m()).into(),
            );
        });

        interval.tick().await;
    }
}

async fn get_key_count(pool: &sqlx::SqlitePool, duration: u16) -> i32 {
    let result = sqlx::query_as::<_, (i64,)>(
        r#"
    SELECT COUNT(*) as count
        FROM keycount WHERE ts_ms > (strftime('%s', 'now') - ?)*1000
    "#,
    )
    .bind(duration)
    .fetch_one(pool)
    .await;

    match result {
        Ok((count,)) => count as i32,
        Err(e) => {
            tracing::error!(error = %e, "can't do query key count");
            0
        }
    }
}

async fn get_wpm(pool: &sqlx::SqlitePool, duration: u16) -> i32 {
    let result = sqlx::query_as::<_, (f64,)>(r#"
        WITH gaps AS (
            SELECT
            ts_ms,
            CASE WHEN ts_ms - LAG(ts_ms) OVER (ORDER BY ts_ms) > 5000 THEN 1 ELSE 0 END as new_session
            FROM keycount
            WHERE ts_ms BETWEEN (strftime('%s', 'now') - ?)*1000 AND (strftime('%s', 'now') - 0)*1000
        ),
        sessions AS (
            SELECT
            ts_ms,
            SUM(new_session) OVER (ORDER BY ts_ms) as session_id
            FROM gaps
        ),
        session_stats AS (
            SELECT
            session_id,
            COUNT(*) as total_keys,
            (MAX(ts_ms) - MIN(ts_ms)) / 1000.0 as duration_secs
            FROM sessions
            GROUP BY session_id
            HAVING duration_secs > 10 -- min session duration (10sec)
        )
        SELECT
        COALESCE(
            AVG(total_keys * 60.0 / duration_secs / 5.0),
            0.0
        ) as avg_wpm
        FROM session_stats
    "#)
        .bind(duration)
        .fetch_one(pool)
        .await;

    match result {
        Ok((avg_wpm,)) => avg_wpm as i32,
        Err(e) => {
            tracing::error!(error = %e, "can't do query wpm");
            0
        }
    }
}

async fn get_keycount_details(pool: &sqlx::SqlitePool, duration: u16, group: u16) -> Vec<i32> {
    let result = sqlx::query_as::<_, (i32,)>(
        r#"
        SELECT COUNT(*) as count
        FROM keycount WHERE ts_ms > (strftime('%s', 'now') - ?)*1000
        GROUP BY ts_ms / (?*1000);
    "#,
    )
    .bind(duration)
    .bind(group)
    .fetch_all(pool)
    .await;

    match result {
        Ok(rows) => rows.iter().map(|row| row.0 as i32).collect(),
        Err(e) => {
            tracing::error!(error = %e, "can't do query key count details");
            vec![]
        }
    }
}

fn generate_path(data: Vec<i32>, width: f32, height: f32) -> String {
    if data.is_empty() {
        return String::new();
    }
    if data.len() == 1 {
        return String::new();
    }

    let max = data.iter().cloned().fold(i32::MIN, i32::max).max(1);
    let step = width / (data.len() - 1) as f32;

    let points: Vec<(f32, f32)> = data
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = i as f32 * step;
            let y = height - (v as f32 / max as f32) * height;
            (x, y)
        })
        .collect();

    let mut path = format!("M 0 {} ", height); // bas gauche
    path.push_str(&format!("L {} {} ", points[0].0, points[0].1));

    for i in 1..points.len() {
        let cp_x = (points[i - 1].0 + points[i].0) / 2.0;
        path.push_str(&format!(
            "C {} {} {} {} {} {} ",
            cp_x,
            points[i - 1].1,
            cp_x,
            points[i].1,
            points[i].0,
            points[i].1
        ));
    }

    path.push_str(&format!("L {} {} Z", width, height)); // bas droite + close
    path
}
