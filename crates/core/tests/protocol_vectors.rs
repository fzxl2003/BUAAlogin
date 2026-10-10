use buaa_core::protocol::login_fields;
#[test]
fn fixed_srun_javascript_vectors() {
    let fixtures: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/protocol.json")).unwrap();
    for case in fixtures.as_array().unwrap() {
        let field = |name: &str| case[name].as_str().unwrap();
        let result = login_fields(
            field("username"),
            field("password"),
            field("ip"),
            field("acid"),
            field("token"),
        );
        assert_eq!(result.password, field("encrypted_password"));
        assert_eq!(result.info, field("info"));
        assert_eq!(result.checksum, field("checksum"));
    }
}
