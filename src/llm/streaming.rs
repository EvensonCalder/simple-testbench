use std::{future::Future, time::Duration};

use anyhow::{Context, anyhow};
use futures_util::StreamExt;

pub(crate) fn block_on_http<T>(
    future: impl Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .context("failed to build HTTP runtime")?;

    runtime.block_on(future)
}

pub(crate) async fn read_sse_response<F>(
    response: reqwest::Response,
    idle_timeout: Duration,
    mut handle_data: F,
) -> anyhow::Result<()>
where
    F: FnMut(&str) -> anyhow::Result<bool>,
{
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut data = Vec::new();

    loop {
        let next_chunk = tokio::time::timeout(idle_timeout, stream.next())
            .await
            .map_err(|_| {
                anyhow!(
                    "stream response timed out after {} seconds",
                    idle_timeout.as_secs()
                )
            })?;

        let Some(chunk) = next_chunk else {
            process_remaining_buffer(&mut buffer, &mut data, &mut handle_data)?;
            return dispatch_event(&mut data, &mut handle_data).map(|_| ());
        };

        let chunk = chunk.context("failed to read streaming response chunk")?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(newline_index) = buffer.find('\n') {
            let line = buffer[..newline_index].to_string();
            buffer.drain(..=newline_index);

            if process_sse_line(&line, &mut data, &mut handle_data)? {
                return Ok(());
            }
        }
    }
}

fn process_remaining_buffer<F>(
    buffer: &mut String,
    data: &mut Vec<String>,
    handle_data: &mut F,
) -> anyhow::Result<bool>
where
    F: FnMut(&str) -> anyhow::Result<bool>,
{
    if buffer.is_empty() {
        return Ok(false);
    }

    let line = std::mem::take(buffer);
    process_sse_line(&line, data, handle_data)
}

fn process_sse_line<F>(
    line: &str,
    data: &mut Vec<String>,
    handle_data: &mut F,
) -> anyhow::Result<bool>
where
    F: FnMut(&str) -> anyhow::Result<bool>,
{
    let line = line.trim_end_matches('\r');
    if line.is_empty() {
        return dispatch_event(data, handle_data);
    }

    if let Some(value) = line.strip_prefix("data:") {
        data.push(value.strip_prefix(' ').unwrap_or(value).to_string());
    }

    Ok(false)
}

fn dispatch_event<F>(data: &mut Vec<String>, handle_data: &mut F) -> anyhow::Result<bool>
where
    F: FnMut(&str) -> anyhow::Result<bool>,
{
    if data.is_empty() {
        return Ok(false);
    }

    let payload = data.join("\n");
    data.clear();

    handle_data(&payload)
}
