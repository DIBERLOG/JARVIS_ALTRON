use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::{accept_hdr_async_with_config, tungstenite::{Message, protocol::WebSocketConfig, handshake::server::{Request,Response,ErrorResponse}}};
use rand::RngCore;
use subtle::ConstantTimeEq;
use std::time::Duration;

const MAX_MESSAGE:usize=64*1024;
use crate::ipc_credentials::credential_path;
pub use crate::ipc_credentials::read_credentials;
fn create_credentials()->Result<String,String>{
    use std::io::Write;
    let path=credential_path()?;let mut bytes=[0u8;32];rand::rngs::OsRng.fill_bytes(&mut bytes);let token:String=bytes.iter().map(|b|format!("{b:02x}")).collect();
    let mut file=tempfile::NamedTempFile::new_in(path.parent().ok_or("IPC directory unavailable")?).map_err(|_|"IPC secret creation failed")?;
    file.write_all(token.as_bytes()).and_then(|_|file.as_file().sync_all()).map_err(|_|"IPC secret save failed")?;file.persist(path).map_err(|_|"IPC secret replacement failed")?;Ok(token)
}
fn allowed_origin(origin:Option<&str>)->bool{match origin{None=>true,Some("tauri://localhost"|"http://tauri.localhost"|"https://tauri.localhost")=>true,Some("http://localhost:1420"|"http://127.0.0.1:1420")=>cfg!(debug_assertions),_=>false}}
fn authenticated(text:&str,expected:&str)->bool{
    #[derive(serde::Deserialize)]#[serde(deny_unknown_fields)]struct Auth{action:String,token:String}
    if text.len()>512{return false;}serde_json::from_str::<Auth>(text).is_ok_and(|a|a.action=="authenticate"&&bool::from(a.token.as_bytes().ct_eq(expected.as_bytes())))
}

use super::events::{IpcAction, IpcEvent};

pub const IPC_PORT: u16 = 9712;
pub const IPC_ADDR: &str = "127.0.0.1";

static BROADCAST_TX: OnceCell<broadcast::Sender<IpcEvent>> = OnceCell::new();
static ACTION_HANDLER: OnceCell<Arc<RwLock<Option<Box<dyn Fn(IpcAction) + Send + Sync>>>>> = OnceCell::new();

// Initialize the IPC broadcast channel
pub fn init() -> broadcast::Sender<IpcEvent> {
    if let Some(tx) = BROADCAST_TX.get() {
        return tx.clone();
    }

    let (tx, _) = broadcast::channel::<IpcEvent>(32);
    BROADCAST_TX.set(tx.clone()).ok();
    ACTION_HANDLER.set(Arc::new(RwLock::new(None))).ok();
    
    info!("IPC: Broadcast channel initialized");
    tx
}

// Send event to all connected clients
pub fn send(event: IpcEvent) {
    if let Some(tx) = BROADCAST_TX.get() {
        match tx.send(event.clone()) {
            Ok(n) => {
                if n > 0 {
                    debug!("IPC: Sent event to {} client(s)", n);
                }
            }
            Err(_) => {
                // no receivers, that's fine
            }
        }
    }
}

// Register handler for incoming actions from GUI
pub fn set_action_handler<F>(handler: F)
where
    F: Fn(IpcAction) + Send + Sync + 'static,
{
    if let Some(h) = ACTION_HANDLER.get() {
        *h.write() = Some(Box::new(handler));
    }
}

fn handle_action(action: IpcAction) {
    debug!("IPC: Received authenticated action");
    
    // handle ping internally
    if matches!(action, IpcAction::Ping) {
        send(IpcEvent::Pong);
        return;
    }
    
    // forward to registered handler
    if let Some(handler_lock) = ACTION_HANDLER.get() {
        let handler = handler_lock.read();
        if let Some(ref h) = *handler {
            h(action);
        }
    }
}

// Start the WebSocket server (blocking)
pub async fn start_server() {
    let addr = format!("{}:{}", IPC_ADDR, IPC_PORT);
    let socket_addr: SocketAddr = addr.parse().expect("Invalid IPC address");

    let listener = match TcpListener::bind(&socket_addr).await {
        Ok(l) => {
            info!("IPC: WebSocket server listening on ws://{}", addr);
            l
        }
        Err(e) => {
            error!("IPC: Failed to bind to {}: {}", addr, e);
            return;
        }
    };

    let token=match create_credentials(){Ok(token)=>Arc::new(token),Err(error)=>{error!("IPC: {}",error);return;}};
    let slots=Arc::new(tokio::sync::Semaphore::new(16));
    // notify that we're ready
    send(IpcEvent::Started);

    while let Ok((stream, peer_addr)) = listener.accept().await {
        info!("IPC: Client connecting from {}", peer_addr);
        
        let Ok(permit)=slots.clone().try_acquire_owned() else{continue;};
        let token=token.clone();tokio::spawn(async move{let _permit=permit;handle_client(stream,peer_addr,token).await;});
    }
}

async fn handle_client(
    stream: TcpStream,
    peer_addr: SocketAddr,
    token:Arc<String>,
) {
    let mut config=WebSocketConfig::default();config.max_message_size=Some(MAX_MESSAGE);config.max_frame_size=Some(MAX_MESSAGE);
    let handshake=accept_hdr_async_with_config(stream,|request:&Request,response:Response|->Result<Response,ErrorResponse>{
        let origin=request.headers().get("origin");let parsed=origin.and_then(|v|v.to_str().ok());
        if (origin.is_some()&&parsed.is_none())||!allowed_origin(parsed){let mut denied=ErrorResponse::new(Some("Forbidden origin".into()));*denied.status_mut()=tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;return Err(denied);}Ok(response)
    },Some(config));
    let mut ws_stream = match tokio::time::timeout(Duration::from_secs(5),handshake).await {
        Ok(Ok(ws)) => {
            info!("IPC: Client connected: {}", peer_addr);
            ws
        }
        _ => {
            debug!("IPC: WebSocket handshake rejected for {}", peer_addr);
            return;
        }
    };
    match tokio::time::timeout(Duration::from_secs(5),ws_stream.next()).await{
        Ok(Some(Ok(Message::Text(text)))) if authenticated(&text,&token)=>{},
        _=>{let _=ws_stream.close(None).await;return;}
    }
    if ws_stream.send(Message::Text("{\"event\":\"authenticated\"}".into())).await.is_err(){return;}
    let mut event_rx=BROADCAST_TX.get().expect("IPC not initialized").subscribe();
    let mut rate_start=std::time::Instant::now();let mut received=0usize;

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    loop {
        tokio::select! {
            // forward events to client
            event_result = event_rx.recv() => {
                match event_result {
                    Ok(event) => {
                        let json = match serde_json::to_string(&event) {
                            Ok(j) => j,
                            Err(e) => {
                                error!("IPC: Failed to serialize event: {}", e);
                                continue;
                            }
                        };

                        if ws_tx.send(Message::Text(json.into())).await.is_err() {
                            info!("IPC: Client {} disconnected (send failed)", peer_addr);
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("IPC: Client {} lagged {} events", peer_addr, n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("IPC: Broadcast channel closed");
                        break;
                    }
                }
            }

            // receive messages from client
            msg_result = ws_rx.next() => {
                match msg_result {
                    Some(Ok(Message::Text(text))) => {
                        if rate_start.elapsed()>=Duration::from_secs(1){rate_start=std::time::Instant::now();received=0;}
                        received+=1;if received>32{break;}
                        match serde_json::from_str::<IpcAction>(&text) {
                            Ok(action) => handle_action(action),
                            Err(e) => {
                                debug!("IPC: Invalid action from {} ({:?})", peer_addr, e.classify());
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if ws_tx.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("IPC: Client {} sent close frame", peer_addr);
                        break;
                    }
                    Some(Err(e)) => {
                        error!("IPC: Error receiving from {}: {}", peer_addr, e);
                        break;
                    }
                    None => {
                        info!("IPC: Client {} stream ended", peer_addr);
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    info!("IPC: Client disconnected: {}", peer_addr);
}

pub fn has_clients() -> bool {
    if let Some(tx) = BROADCAST_TX.get() {
        tx.receiver_count() > 0
    } else {
        false
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    async fn endpoint(token:&str)->(String,tokio::task::JoinHandle<()>){
        init();let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();let token=Arc::new(token.to_string());
        let task=tokio::spawn(async move{let (socket,peer)=listener.accept().await.unwrap();handle_client(socket,peer,token).await;});
        (format!("ws://{address}"),task)
    }
    #[tokio::test]
    async fn client_needs_secret_before_commands_or_events(){
        let (url,task)=endpoint("test-secret").await;let(mut client,_)=tokio_tungstenite::connect_async(url).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(100),client.next()).await.is_err());
        client.send(Message::Text("{\"action\":\"text_command\",\"text\":\"must-not-execute\"}".into())).await.unwrap();
        assert!(!matches!(tokio::time::timeout(Duration::from_secs(2),client.next()).await,Ok(Some(Ok(Message::Text(_))))));task.await.unwrap();
        let(url,task)=endpoint("test-secret").await;let(mut client,_)=tokio_tungstenite::connect_async(url).await.unwrap();
        client.send(Message::Text("{\"action\":\"authenticate\",\"token\":\"test-secret\"}".into())).await.unwrap();
        let message=client.next().await.unwrap().unwrap().into_text().unwrap();assert!(message.contains("authenticated"));
        client.send(Message::Text("{\"action\":\"ping\"}".into())).await.unwrap();let message=client.next().await.unwrap().unwrap().into_text().unwrap();assert!(message.contains("pong"));
        client.close(None).await.unwrap();task.await.unwrap();
    }
    #[tokio::test]
    async fn rejects_foreign_browser_origin(){
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let(url,task)=endpoint("test-secret").await;let mut request=url.into_client_request().unwrap();request.headers_mut().insert("origin","https://example.invalid".parse().unwrap());
        let result=tokio_tungstenite::connect_async(request).await;
        assert!(matches!(result,Err(tokio_tungstenite::tungstenite::Error::Http(response)) if response.status().as_u16()==403));task.await.unwrap();
        assert!(!authenticated("{\"action\":\"authenticate\",\"token\":\"wrong\"}","test-secret"));
    }
}
