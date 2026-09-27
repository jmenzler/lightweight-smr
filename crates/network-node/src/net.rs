//! Async frame reader; trailing padding stays unread, safe only because padded frames ride one-shot connections.

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

pub async fn read_frame(stream: &mut TcpStream, max_frame_bytes: u32) -> Result<Vec<u8>, String> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| e.to_string())?;
    let body_len = u32::from_le_bytes(len_buf) as usize;
    if body_len + 4 > max_frame_bytes as usize {
        return Err(format!(
            "declared body of {body_len} bytes exceeds the {max_frame_bytes}-byte cap"
        ));
    }
    let mut frame = vec![0u8; 4 + body_len];
    frame[..4].copy_from_slice(&len_buf);
    stream
        .read_exact(&mut frame[4..])
        .await
        .map_err(|e| e.to_string())?;
    Ok(frame)
}
