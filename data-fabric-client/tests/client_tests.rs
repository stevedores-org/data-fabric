use data_fabric_client::{Client, ClientConfig};
use std::net::TcpListener;
use std::io::{Read, Write};
use tokio::task;

#[tokio::test]
async fn test_client_headers_and_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    
    // Spawn mock server thread
    let server_handle = task::spawn_blocking(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0; 2048];
        let bytes_read = stream.read(&mut buffer).unwrap();
        let request_str = String::from_utf8_lossy(&buffer[..bytes_read]);
        
        // Assert client headers
        assert!(request_str.contains("x-tenant-id: test-tenant"));
        assert!(request_str.contains("x-tenant-role: builder"));
        assert!(request_str.contains("cf-access-client-id: id-123"));
        assert!(request_str.contains("cf-access-client-secret: secret-456"));
        
        let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"service\":\"data-fabric\",\"status\":\"ok\",\"mission\":\"testing\"}";
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    });

    let config = ClientConfig {
        base_url: format!("http://127.0.0.1:{}", port),
        tenant_id: "test-tenant".to_string(),
        tenant_role: "builder".to_string(),
        cf_client_id: Some("id-123".to_string()),
        cf_client_secret: Some("secret-456".to_string()),
    };

    let client = Client::new(config);
    let health = client.health().await.unwrap();

    assert_eq!(health.service, "data-fabric");
    assert_eq!(health.status, "ok");
    assert_eq!(health.mission, "testing");

    server_handle.await.unwrap();
}
