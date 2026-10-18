use super::authorization::{NativeStream, WriteScope, Writing};
use super::{CodexError, NativeApproval, NativeEvent, Work, normalize};
use futures_util::{SinkExt, StreamExt};
use qgenie_remote_proto::{MAX_JSON_BYTES, wire::ApprovalDecision};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    pin::Pin,
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::{Instant, timeout},
};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
struct Pending {
    done: oneshot::Sender<Result<Value, CodexError>>,
    deadline: Instant,
}
struct ApprovalState {
    approval: NativeApproval,
    answered: bool,
}
type NativeSink = futures_util::stream::SplitSink<WebSocketStream<NativeStream>, Message>;
type NativeWrite = Pin<Box<dyn Future<Output = (NativeSink, Result<(), CodexError>)> + Send>>;
struct Sending {
    future: NativeWrite,
    done: Option<oneshot::Sender<Result<Value, CodexError>>>,
    // Retain original authority and byte ownership until success or driver drop.
    _scope: Writing,
    _bytes: Option<tokio::sync::OwnedSemaphorePermit>,
}
impl Sending {
    fn begin(
        mut input: NativeSink,
        message: Message,
        scope: Writing,
        done: Option<oneshot::Sender<Result<Value, CodexError>>>,
        bytes: Option<tokio::sync::OwnedSemaphorePermit>,
    ) -> Self {
        Self {
            future: Box::pin(async move {
                let result = timeout(Duration::from_secs(5), input.send(message))
                    .await
                    .map_err(|_| CodexError::Unknown)
                    .and_then(|result| result.map_err(|_| CodexError::Unknown));
                (input, result)
            }),
            done,
            _scope: scope,
            _bytes: bytes,
        }
    }
}
fn json_message(body: &Value) -> Result<Message, CodexError> {
    let text = serde_json::to_string(body).map_err(|_| CodexError::Protocol)?;
    if text.len() > MAX_JSON_BYTES {
        return Err(CodexError::Protocol);
    }
    Ok(Message::Text(text.into()))
}
pub(super) async fn run(
    socket: WebSocketStream<NativeStream>,
    mut work: mpsc::Receiver<Work>,
    events: mpsc::Sender<NativeEvent>,
    mut stopping: oneshot::Receiver<()>,
    relay: watch::Sender<bool>,
    mut process_running: watch::Receiver<bool>,
    scope: WriteScope,
) -> Result<(), CodexError> {
    let (input, mut output) = socket.split();
    let mut input = Some(input);
    let mut sending: Option<Sending> = None;
    let mut protocol = VecDeque::new();
    let mut pending = BTreeMap::<u64, Pending>::new();
    let mut approvals = BTreeMap::<String, ApprovalState>::new();
    let mut resolved = VecDeque::<(String, bool)>::new();
    let mut usage = BTreeMap::<String, ((u64, u64), (u64, u64))>::new();
    let mut next = 1u64;
    let mut deadlines = tokio::time::interval(Duration::from_millis(100));
    let result=async {
        loop {
            tokio::select! {
                _=&mut stopping=>return Ok(()),
                _=process_running.changed()=>return Err(CodexError::Process),
                written=async {sending.as_mut().unwrap().future.as_mut().await},if sending.is_some()=> {
                    let (writer,result)=written;
                    // On failure return while the original scope still exists;
                    // no reader/new grant may flush uncertain buffered bytes.
                    result?;
                    input=Some(writer);
                    if let Some(done)=sending.take().unwrap().done {let _=done.send(Ok(json!({})));}
                },
                _=async {},if sending.is_none() && !protocol.is_empty()=> {
                    sending=Some(Sending::begin(input.take().unwrap(),protocol.pop_front().unwrap(),scope.enter(None),None,None));
                },
                Some(item)=work.recv(),if sending.is_none() && protocol.is_empty()=> {
                    let mut body=item.body;
                    let request=body.get("method").is_some() && body.get("id").is_some();
                    let response=body.get("method").is_none() && body.get("id").is_some();
                    if request && pending.len()>=64 { let _=item.done.send(Err(CodexError::NotSent));continue; }
                    if response {
                        let key=body["id"].to_string();
                        if let Some(state)=approvals.get_mut(&key) {
                            if state.answered { let _=item.done.send(Ok(json!({"application":"unknown"})));continue; }
                            if !state.approval.allowed_decisions.iter().any(|allowed| decision(&state.approval,allowed).ok().is_some_and(|value| body["result"]["decision"]==value)) {
                                let _=item.done.send(Err(CodexError::Unsupported));continue;
                            }
                            // Poison before the write: a partial write must not permit redelivery.
                            state.answered=true;
                        } else {
                            let answered=resolved.iter().find(|(id,_)| id==&key).map(|(_,answered)| *answered);
                            let _=item.done.send(match answered { Some(false)=>Ok(json!({"application":"cancelled"})),Some(true)=>Ok(json!({"application":"unknown"})),None=>Err(CodexError::Unsupported) });
                            continue;
                        }
                    }
                    if request { body["id"]=json!(next); }
                    let message=json_message(&body)?;
                    let done=if request {
                        // A response can arrive while flush completion is pending.
                        pending.insert(next,Pending { done:item.done,deadline:Instant::now()+Duration::from_secs(15) });
                        next=next.checked_add(1).ok_or(CodexError::Protocol)?;
                        None
                    } else {Some(item.done)};
                    sending=Some(Sending::begin(input.take().unwrap(),message,scope.enter(item.authorization),done,Some(item._bytes)));
                },
                message=output.next()=> {
                    let message=match message.ok_or(CodexError::Process)?.map_err(|_| CodexError::Protocol)? {
                        Message::Text(text)=>qgenie_remote_proto::decode_json::<Value>(text.as_bytes()).map_err(|_| CodexError::Protocol)?,
                        Message::Ping(bytes)=> {if protocol.len()>=4{return Err(CodexError::SlowConsumer);}protocol.push_back(Message::Pong(bytes));continue; },
                        Message::Pong(_)=>continue,
                        Message::Close(_)=>return Err(CodexError::Process),
                        _=>return Err(CodexError::Protocol),
                    };
                    if let Some(method)=message["method"].as_str() {
                        let params=&message["params"];
                        if method=="remoteControl/status/changed" {
                            require_disabled(params)?;
                            let _=relay.send(true);continue;
                        }
                        if method=="serverRequest/resolved" {
                            let key=params["requestId"].to_string();
                            let state=approvals.remove(&key);
                            if resolved.len()==128 { resolved.pop_front(); }
                            let answered=state.is_some_and(|state| state.answered) || resolved.iter().any(|(id,answered)| id==&key && *answered);
                            resolved.push_back((key.clone(),answered));
                            emit(&events,NativeEvent::ApprovalResolved { native_id:key,thread:normalize::text(params,"threadId")? })?;
                            continue;
                        }
                        if method=="turn/started" {
                            let thread=normalize::text(params,"threadId")?;
                            if usage.len()>=64 && !usage.contains_key(&thread) { return Err(CodexError::SlowConsumer); }
                            let totals=usage.get(&thread).map_or((0,0),|state| state.1);
                            usage.insert(thread,(totals,totals));
                        }
                        if method=="thread/tokenUsage/updated" {
                            let thread=normalize::text(params,"threadId")?;
                            if let Some(state)=usage.get_mut(&thread) {
                                state.1=(params["tokenUsage"]["total"]["inputTokens"].as_u64().ok_or(CodexError::Protocol)?,params["tokenUsage"]["total"]["outputTokens"].as_u64().ok_or(CodexError::Protocol)?);
                            }
                        }
                        if message.get("id").is_some() {
                            if approvals.len()>=32 { return Err(CodexError::SlowConsumer); }
                            if let Some(approval)=normalize::approval(method,&message)? {
                                let key=approval.native_id.clone();
                                if approvals.contains_key(&key) { return Err(CodexError::Protocol); }
                                approvals.insert(key,ApprovalState { approval:approval.clone(),answered:false });
                                emit(&events,NativeEvent::Approval(approval))?;
                            } else {
                                // Unknown approval/tool methods fail explicitly instead of hanging or accepting.
                                if protocol.len()>=4 || message["id"].to_string().len()>512 {return Err(CodexError::SlowConsumer);}
                                protocol.push_back(json_message(&json!({"id":message["id"],"error":{"code":-32601,"message":"unsupported native request"}}))?);
                            }
                        } else if let Some(mut event)=normalize::event(method,params)? {
                            if let NativeEvent::Event { thread,event:qgenie_remote_proto::wire::AgentEvent::TurnCompleted { input_tokens,output_tokens,.. } }=&mut event
                                && let Some((base,total))=usage.get(thread) {
                                *input_tokens=qgenie_remote_proto::Counter(total.0.saturating_sub(base.0));
                                *output_tokens=qgenie_remote_proto::Counter(total.1.saturating_sub(base.1));
                            }
                            emit(&events,event)?;
                        }
                    } else if let Some(id)=message["id"].as_u64() {
                        let Some(waiter)=pending.remove(&id) else { continue; };
                        let result=if let Some(error)=message.get("error") { Err(CodexError::Rejected(error["code"].as_i64().unwrap_or(-32603))) } else { message.get("result").cloned().ok_or(CodexError::Protocol) };
                        let _=waiter.done.send(result);
                    } else { return Err(CodexError::Protocol); }
                },
                _=deadlines.tick()=> {
                    let expired:Vec<_>=pending.iter().filter(|(_,request)| request.deadline<=Instant::now()).map(|(id,_)| *id).collect();
                    for id in expired { if let Some(waiter)=pending.remove(&id) { let _=waiter.done.send(Err(CodexError::Unknown)); } }
                },
            }
        }
    }.await;
    // Dropped pending receipts mean unknown. There is deliberately no native mutation retry.
    result
}
fn require_disabled(params: &Value) -> Result<(), CodexError> {
    if params["status"] == "disabled" {
        Ok(())
    } else {
        Err(CodexError::Relay)
    }
}
fn emit(events: &mpsc::Sender<NativeEvent>, event: NativeEvent) -> Result<(), CodexError> {
    events.try_send(event).map_err(|_| CodexError::SlowConsumer)
}
pub(super) fn decision(
    approval: &NativeApproval,
    decision: &ApprovalDecision,
) -> Result<&'static str, CodexError> {
    let key = serde_json::to_string(decision).map_err(|_| CodexError::Protocol)?;
    if !approval
        .allowed_decisions
        .iter()
        .any(|allowed| serde_json::to_string(allowed).ok().as_ref() == Some(&key))
    {
        return Err(CodexError::Unsupported);
    }
    match decision {
        ApprovalDecision::AllowOnce => Ok("accept"),
        ApprovalDecision::AllowSession => Ok("acceptForSession"),
        ApprovalDecision::Deny => Ok("cancel"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_events_drain_while_original_authority_defers_a_write() {
        use std::sync::{
            Arc,
            atomic::{AtomicU8, AtomicUsize, Ordering},
        };
        struct Busy {
            mode: AtomicU8,
            attempts: AtomicUsize,
        }
        impl crate::NativeEffectAuthorization for Busy {
            fn perform(
                &self,
                effect: &mut dyn FnMut() -> std::io::Result<usize>,
            ) -> std::io::Result<usize> {
                self.attempts.fetch_add(1, Ordering::SeqCst);
                match self.mode.load(Ordering::SeqCst) {
                    0 => Err(std::io::ErrorKind::WouldBlock.into()),
                    1 => effect(),
                    _ => Err(std::io::ErrorKind::PermissionDenied.into()),
                }
            }
        }
        for admit in [false, true] {
            let (client, peer) = tokio::net::UnixStream::pair().unwrap();
            let scope = WriteScope::default();
            let client = WebSocketStream::from_raw_socket(
                NativeStream::new(client, scope.clone()),
                tokio_tungstenite::tungstenite::protocol::Role::Client,
                None,
            )
            .await;
            let mut peer = WebSocketStream::from_raw_socket(
                peer,
                tokio_tungstenite::tungstenite::protocol::Role::Server,
                None,
            )
            .await;
            let (commands, work) = mpsc::channel(4);
            let (events, mut observed) = mpsc::channel(4);
            let (stop, stopping) = oneshot::channel();
            let (relay, _) = watch::channel(false);
            let (_process, running) = watch::channel(true);
            let task = tokio::spawn(run(client, work, events, stopping, relay, running, scope));
            let busy = Arc::new(Busy {
                mode: AtomicU8::new(0),
                attempts: AtomicUsize::new(0),
            });
            let (done, result) = oneshot::channel();
            commands
                .send(Work {
                    body: json!({"method":"initialized","params":{}}),
                    done,
                    authorization: Some(busy.clone()),
                    _bytes: Arc::new(tokio::sync::Semaphore::new(1))
                        .acquire_owned()
                        .await
                        .unwrap(),
                })
                .await
                .unwrap();
            timeout(Duration::from_secs(1), async {
                while busy.attempts.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            peer.send(json_message(&json!({"method":"item/agentMessage/delta","params":{"threadId":"owned-thread","itemId":"message","delta":"output while control waits"}})).unwrap()).await.unwrap();
            let event = timeout(Duration::from_millis(500), observed.recv())
                .await
                .expect("native event drain blocked on authority")
                .unwrap();
            assert!(
                matches!(event,NativeEvent::Event{event:qgenie_remote_proto::wire::AgentEvent::MessageDelta{text,..},..} if text=="output while control waits")
            );
            busy.mode.store(if admit { 1 } else { 2 }, Ordering::SeqCst);
            let result = timeout(Duration::from_secs(1), result).await.unwrap();
            if admit {
                result.unwrap().unwrap();
                let Message::Text(frame) = timeout(Duration::from_secs(1), peer.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
                else {
                    panic!("one native write");
                };
                assert_eq!(
                    qgenie_remote_proto::decode_json::<Value>(frame.as_bytes()).unwrap()["method"],
                    "initialized"
                );
                stop.send(()).unwrap();
                task.await.unwrap().unwrap();
            } else {
                assert!(result.is_err() || result.unwrap().is_err());
                assert!(task.await.unwrap().is_err());
                let read = timeout(Duration::from_secs(1), peer.next()).await.unwrap();
                assert!(
                    !matches!(read, Some(Ok(Message::Text(_)))),
                    "expired authority flushed a buffered native frame"
                );
            }
        }
    }
    #[test]
    fn relay_drift_is_refused_and_native_decisions_respect_offered_options() {
        for status in ["connecting", "connected", "errored", ""] {
            assert!(matches!(
                require_disabled(&json!({"status":status})),
                Err(CodexError::Relay)
            ));
        }
        assert!(require_disabled(&json!({"status":"disabled"})).is_ok());
        let message: Value = serde_json::from_str(include_str!(
            "../../../../conformance/codex/0.159.2/approval.json"
        ))
        .unwrap();
        let approval = normalize::approval("item/commandExecution/requestApproval", &message)
            .unwrap()
            .unwrap();
        assert_eq!(
            decision(&approval, &ApprovalDecision::AllowOnce).unwrap(),
            "accept"
        );
        assert_eq!(
            decision(&approval, &ApprovalDecision::Deny).unwrap(),
            "cancel"
        );
        assert!(matches!(
            decision(&approval, &ApprovalDecision::AllowSession),
            Err(CodexError::Unsupported)
        ));
        assert!(!format!("{approval:?}").contains("touch"));
    }
}
