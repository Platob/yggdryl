#[test]
fn get_all_key_lifetime_probe() {
    let url = yggdryl::Url::from_str("https://e.com/t?a=1&a=2").unwrap();
    let params = url.parameters(false).unwrap();
    let collected: Vec<&str> = {
        let key = String::from("a");
        params.get_all(&key).collect()
    };
    assert_eq!(collected, ["1", "2"]);
}
