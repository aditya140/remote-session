//! Test the actual pinned vendor's cwd handling before enabling managed remote starts.
use super::*;
use std::os::{
    fd::AsRawFd,
    unix::fs::{OpenOptionsExt, PermissionsExt},
};

#[tokio::test]
#[ignore = "requires actual QGR_CODEX_BINARY; descriptor cwd equivalence qualification"]
async fn actual_pinned_codex_retains_descriptor_cwd_through_launch_and_thread_path_replacement() {
    let binary = PathBuf::from(std::env::var_os("QGR_CODEX_BINARY").expect("set QGR_CODEX_BINARY"));
    let temp = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let alias = root.join("chosen");
    let held = root.join("held_before_thread");
    let final_path = root.join("held_after_thread");
    let outside = root.join("outside");
    std::fs::create_dir(&alias).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let directory = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&alias)
        .unwrap();
    std::fs::rename(&alias, &held).unwrap();
    std::os::unix::fs::symlink(&outside, &alias).unwrap();
    let descriptor = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        directory.as_raw_fd()
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut model = tokio::task::JoinSet::new();
    model.spawn(async move {
        for index in 1..=2 {
            let (mut stream,_)=listener.accept().await.unwrap();let mut header=Vec::new();
            while !header.ends_with(b"\r\n\r\n"){assert!(header.len()<32768);header.push(stream.read_u8().await.unwrap());}
            let header=String::from_utf8(header).unwrap();let length=header.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|n|n.trim().parse::<usize>().unwrap())).unwrap();
            assert!(length<2*1024*1024);let mut body=vec![0;length];stream.read_exact(&mut body).await.unwrap();
            let item=if index==1 {json!({"type":"function_call","id":"fc_cwd","call_id":"call_cwd","name":"exec_command","arguments":json!({"cmd":"touch native_descriptor_marker","max_output_chars":1000}).to_string()})}else{json!({"type":"message","id":"msg_cwd","role":"assistant","status":"completed","content":[{"type":"output_text","text":"descriptor proof complete","annotations":[]}]})};
            let response=format!("cwd_{index}");let events=[json!({"type":"response.created","response":{"id":response}}),json!({"type":"response.output_item.added","output_index":0,"item":item}),json!({"type":"response.output_item.done","output_index":0,"item":item}),json!({"type":"response.completed","response":{"id":response,"status":"completed","output":[item],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}})];
            let mut output="HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n".to_owned();for event in events {output.push_str(&format!("event: {}\ndata: {}\n\n",event["type"].as_str().unwrap(),event));}stream.write_all(output.as_bytes()).await.unwrap();
        }
    });
    let mut configuration = config(&binary, &root, port);
    configuration.cwd = descriptor.clone();
    let mut adapter = Codex::launch_at(configuration, directory).await.unwrap();
    let thread = adapter.start_thread(&descriptor).await.unwrap();
    // A canonical path retained at thread/start would now resolve to the outside directory.
    std::fs::rename(&held, &final_path).unwrap();
    std::os::unix::fs::symlink(&outside, &held).unwrap();
    adapter
        .prompt(
            &thread,
            "Create the relative descriptor marker for this controlled fixture.",
        )
        .await
        .unwrap();
    let mut approvals = 0;
    let mut tool_success = false;
    loop {
        match next(&mut adapter).await {
            NativeEvent::Approval(approval) => {
                approvals += 1;
                adapter
                    .answer(&approval, ApprovalDecision::AllowOnce)
                    .await
                    .unwrap();
            }
            NativeEvent::Event {
                event: AgentEvent::ToolCompleted { success, .. },
                ..
            } => tool_success = success,
            NativeEvent::Event {
                event: AgentEvent::TurnCompleted { .. },
                ..
            } => break,
            _ => {}
        }
    }
    adapter.shutdown().await.unwrap();
    while let Some(result) = model.join_next().await {
        result.unwrap();
    }
    assert_eq!(approvals, 1);
    assert!(
        tool_success,
        "native relative tool did not succeed using the retained descriptor cwd"
    );
    assert!(
        final_path.join("native_descriptor_marker").exists(),
        "native command did not retain the original cwd inode"
    );
    assert!(
        !outside.join("native_descriptor_marker").exists(),
        "native command followed a replaced cwd path"
    );
    assert!(
        !descriptor.exists(),
        "native owner retained cwd descriptor after process reap"
    );
}
