/// Tests for stdlib/prelude.tri using the tri_test_helpers macros.

#[macro_use]
extern crate tricti;

#[cfg(test)]
mod tests {
    tri_test_with_prelude!(
        stdlib_prelude_id_and_print_i64,
        r#"
        main :: () => {
            print(123)
            println(id(42))
        }
        "#,
        "123\n42\n"
    );

    tri_test_with_prelude!(
        stdlib_prelude_len_and_streq,
        r#"
        main :: () => {
            println(len("hé"))
            println(streq("a", "a"))
        }
        "#,
        "3\ntrue\n"
    );

    tri_test_with_prelude!(
        stdlib_prelude_math_and_array_helpers,
        r#"
        main :: () => {
            println(clamp_i64(-5, 0, 10))
            println(sign_i64(-42))
            println(sign_i64(0))
            println(sign_i64(11))
            println(is_even_i64(12))
            println(is_odd_i64(13))

            data := [1i64, 2i64, 3i64]
            mapped := array_map(data, (x) => x * 2)
            println(mapped[0])
            println(mapped[1])
            println(mapped[2])

            println(array_all(mapped, (x) => x >= 2))
            println(array_any(mapped, (x) => x > 4))
            println(array_fold(mapped, 0i64, (acc, x) => acc + x))
        }
        "#,
        "0\n-1\n0\n1\ntrue\ntrue\n2\n4\n6\ntrue\ntrue\n12\n"
    );

    tri_test_with_prelude!(
        stdlib_prelude_array_remove_at_guards,
        r#"
        main :: () => {
            data := [1i64, 2i64, 3i64, 4i64]

            trimmed := array_remove_at(data, 2)
            println(len(trimmed))
            println(trimmed[0])
            println(trimmed[1])
            println(trimmed[2])

            unchanged := array_remove_at(data, 10)
            println(len(unchanged))
            println(unchanged[0])
            println(unchanged[1])
            println(unchanged[2])
            println(unchanged[3])

            neg := array_remove_at(data, -1)
            println(len(neg))
            println(neg[0])
        }
        "#,
        "3\n1\n2\n4\n4\n1\n2\n3\n4\n4\n1\n"
    );

    tri_test_with_prelude!(
        stdlib_prelude_modern_error_types,
        r#"
        main :: () => {
            # Test creating different error types
            msg_err := std_error_message_only("simple message")
            panic_err := std_error_panic("panic occurred", some "source info")
            invalid_arg_err := std_error_invalid_argument("param1", "invalid value")
            unsupported_err := std_error_unsupported("experimental feature")

            # Test error message extraction
            println(std_error_message(msg_err))
            println(std_error_message(panic_err))
            println(std_error_message(invalid_arg_err))
            println(std_error_message(unsupported_err))

            # Test result types
            ok_result := std_ok(42)
            err_result := std_err(msg_err)

            # Test result helper functions
            println(std_result_is_ok(ok_result))
            println(std_result_is_ok(err_result))

            println(std_result_unwrap(ok_result))

            match std_result_error(err_result) {
                some error => println(std_error_message(error)),
                none => println("no error"),
            }

            println("Modern error types test completed successfully!")
        }
        "#,
        "simple message\npanic occurred\ninvalid value\nexperimental feature\ntrue\nfalse\n42\nsimple message\nModern error types test completed successfully!\n"
    );
}
