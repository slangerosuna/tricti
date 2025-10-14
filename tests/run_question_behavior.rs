tricti::tri_test_with_prelude!(
    question_mark_propagates_and_unwraps,
    r#"
    use std::prelude
    use std::collections
    use std::collections::vec
    use std::collections
    use std
    use std::core::option

        maybe_even :: (value: i64) -> ?i64 => do
            if value % 2 == 0:
                ret some value
            ret none

        double_even :: (value: i64) -> ?i64 => do
            even_value := maybe_even(value)?
            ret some (even_value * 2)

        main :: () => do
            odd_case := double_even(3)
            match odd_case:
                some value => println(value.to_string()),
                none => println("propagated none"),

            even_case := double_even(4)
            match even_case:
                some value => println(value.to_string()),
                none => println("unexpected none"),
    "#,
    "propagated none\n8\n"
);

tricti::tri_test_with_prelude!(
    question_else_provides_fallback_value,
    r#"
    use std::prelude
    use std::core::option

        maybe_even :: (value: i64) -> ?i64 => do
            if value % 2 == 0:
                ret some value
            ret none

        even_or_default :: (value: i64) -> i64 => do
            even_value := maybe_even(value)? else:
                println("else branch")
                ret -1
            ret even_value

        main :: () => do
            println(even_or_default(3).to_string())
            println(even_or_default(6).to_string())
    "#,
    "else branch\n-1\n6\n"
);
