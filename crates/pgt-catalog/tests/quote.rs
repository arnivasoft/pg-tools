mod common;

#[tokio::test]
async fn quote_identifiers_uses_server_side_quote_ident() {
    for fx in common::fixtures().await {
        let input = ["orders", "Mixed View", "user", "a\"b", "çok", "select", "ok_1"];
        let got = fx.catalog.quote_identifiers(fx.server, input.iter().map(|s| s.to_string()).collect()).await.unwrap();
        assert_eq!(got, vec!["orders", "\"Mixed View\"", "\"user\"", "\"a\"\"b\"", "\"çok\"", "\"select\"", "ok_1"], "PG {}", fx.major().await);
    }
}

#[tokio::test]
async fn quote_identifiers_empty_input_is_empty_output() {
    for fx in common::fixtures().await {
        assert!(fx.catalog.quote_identifiers(fx.server, vec![]).await.unwrap().is_empty());
    }
}
