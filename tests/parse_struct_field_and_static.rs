use peano::parser;

#[test]
fn parse_struct_field_and_static_path() {
    let src = r#"
        Vec2 :: { x: i64, y: i64 }
        len :: (v: Vec2) -> i64 => { 0 }
        main :: () -> i64 => {
            println(Vec2::new)
            0
        }
    "#
    .to_string();
    let _ = parser::parse(src);
}
