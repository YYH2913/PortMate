use super::*;

#[test]
fn filtered_agent_response_exposes_only_allowed_identity_blobs() {
    let mut response = vec![SSH_AGENT_IDENTITIES_ANSWER];
    response.extend_from_slice(&2_u32.to_be_bytes());
    push_ssh_string(&mut response, b"rejected-key").unwrap();
    push_ssh_string(&mut response, b"rejected-comment").unwrap();
    push_ssh_string(&mut response, b"accepted-key").unwrap();
    push_ssh_string(&mut response, b"accepted-comment").unwrap();
    let refs = [IdentityRef {
        id: "accepted".to_string(),
        label: "accepted-comment".to_string(),
        source: IdentitySource::Agent,
        fingerprint_sha256: None,
        path: None,
        secret_ref: None,
    }];

    let (filtered, allowed) = filter_identity_response(&response, &refs).unwrap();
    assert_eq!(allowed, HashSet::from([b"accepted-key".to_vec()]));
    let mut offset = 1_usize;
    assert_eq!(filtered[0], SSH_AGENT_IDENTITIES_ANSWER);
    assert_eq!(read_u32(&filtered, &mut offset), Some(1));
    assert_eq!(
        read_ssh_string(&filtered, &mut offset),
        Some(&b"accepted-key"[..])
    );
    assert_eq!(
        read_ssh_string(&filtered, &mut offset),
        Some(&b"accepted-comment"[..])
    );
    assert_eq!(offset, filtered.len());
}

#[test]
fn filtered_agent_path_matches_exact_comment_bytes() {
    let mut response = vec![SSH_AGENT_IDENTITIES_ANSWER];
    response.extend_from_slice(&2_u32.to_be_bytes());
    push_ssh_string(&mut response, b"rejected-key").unwrap();
    push_ssh_string(&mut response, b"accepted-comment").unwrap();
    push_ssh_string(&mut response, b"accepted-key").unwrap();
    push_ssh_string(&mut response, b"accepted-comment ").unwrap();
    let refs = [IdentityRef {
        id: "accepted".to_string(),
        label: "agent key".to_string(),
        source: IdentitySource::Agent,
        fingerprint_sha256: None,
        path: Some("accepted-comment ".to_string()),
        secret_ref: None,
    }];

    let (filtered, allowed) = filter_identity_response(&response, &refs).unwrap();
    assert_eq!(allowed, HashSet::from([b"accepted-key".to_vec()]));
    let mut offset = 1_usize;
    assert_eq!(filtered[0], SSH_AGENT_IDENTITIES_ANSWER);
    assert_eq!(read_u32(&filtered, &mut offset), Some(1));
    assert_eq!(
        read_ssh_string(&filtered, &mut offset),
        Some(&b"accepted-key"[..])
    );
    assert_eq!(
        read_ssh_string(&filtered, &mut offset),
        Some(&b"accepted-comment "[..])
    );
    assert_eq!(offset, filtered.len());
}

#[test]
fn filtered_agent_parser_rejects_truncated_and_trailing_identity_data() {
    assert!(filter_identity_response(&[SSH_AGENT_IDENTITIES_ANSWER], &[]).is_err());

    let mut response = vec![SSH_AGENT_IDENTITIES_ANSWER];
    response.extend_from_slice(&0_u32.to_be_bytes());
    response.push(0);
    assert!(filter_identity_response(&response, &[]).is_err());
}

#[test]
fn filtered_agent_sign_request_reads_the_exact_key_blob() {
    let mut request = vec![SSH_AGENT_SIGN_REQUEST];
    push_ssh_string(&mut request, b"selected-key").unwrap();
    push_ssh_string(&mut request, b"payload").unwrap();
    request.extend_from_slice(&0_u32.to_be_bytes());
    assert_eq!(sign_request_key_blob(&request), Some(&b"selected-key"[..]));
    assert!(sign_request_key_blob(&[SSH_AGENT_SIGN_REQUEST, 0, 0]).is_none());
}
