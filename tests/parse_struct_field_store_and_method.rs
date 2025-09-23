use peano::parser;

#[test]
fn parse_struct_and_method_syntax() {
    let src = r#"
        point :: { x: i64, y: i64 }
        impl point {
            sum :: (self: &mut point) -> i64 => { self.x + self.y }
        }
        main :: () -> i64 => { 0 }
    "#.to_string();

    let _program = parser::parse(src);
}
