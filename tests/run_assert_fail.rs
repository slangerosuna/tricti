use inkwell::context::Context;
use std::fs;
use std::path::Path;
use std::process::Command;
use tricti::ast::{Expression, Statement};
use tricti::{codegen, parser, semantic};

fn log_if_identifiers(program: &tricti::ast::Program) {
    fn walk_expr(expr: &Expression, path: &mut Vec<String>) {
        match expr {
            Expression::Identifier(name) if name == "if" => {
                eprintln!("found identifier 'if' at {}", path.join(" > "));
                eprintln!("  expr: {expr:#?}");
            }
            Expression::BinaryOp { left, right, .. } => {
                path.push("binary_left".into());
                walk_expr(left, path);
                path.pop();
                path.push("binary_right".into());
                walk_expr(right, path);
                path.pop();
            }
            Expression::UnaryOp { operand, .. } => {
                path.push("unary".into());
                walk_expr(operand, path);
                path.pop();
            }
            Expression::Cast { value, .. } => {
                path.push("cast".into());
                walk_expr(value, path);
                path.pop();
            }
            Expression::Call {
                function,
                arguments,
                ..
            } => {
                path.push("call_fn".into());
                walk_expr(function, path);
                path.pop();
                for (idx, arg) in arguments.iter().enumerate() {
                    path.push(format!("call_arg[{idx}]"));
                    walk_expr(&arg.value, path);
                    path.pop();
                }
            }
            Expression::FieldAccess { object, .. } => {
                path.push("field_obj".into());
                walk_expr(object, path);
                path.pop();
            }
            Expression::Index { object, indices } => {
                path.push("index_obj".into());
                walk_expr(object, path);
                path.pop();
                for (idx, idx_expr) in indices.iter().enumerate() {
                    path.push(format!("index[{idx}]"));
                    walk_expr(idx_expr, path);
                    path.pop();
                }
            }
            Expression::If {
                condition,
                then_branch,
                else_branch,
            } => {
                path.push("if_cond".into());
                walk_expr(condition, path);
                path.pop();
                for (idx, stmt) in then_branch.iter().enumerate() {
                    path.push(format!("if_then[{idx}]"));
                    walk_stmt(stmt, path);
                    path.pop();
                }
                if let Some(else_branch) = else_branch {
                    for (idx, stmt) in else_branch.iter().enumerate() {
                        path.push(format!("if_else[{idx}]"));
                        walk_stmt(stmt, path);
                        path.pop();
                    }
                }
            }
            Expression::IfExpr {
                condition,
                then_expr,
                else_expr,
            } => {
                path.push("ifexpr_cond".into());
                walk_expr(condition, path);
                path.pop();
                path.push("ifexpr_then".into());
                walk_expr(then_expr, path);
                path.pop();
                if let Some(else_expr) = else_expr {
                    path.push("ifexpr_else".into());
                    walk_expr(else_expr, path);
                    path.pop();
                }
            }
            Expression::Loop { body }
            | Expression::Block { statements: body }
            | Expression::UnsafeBlock { statements: body } => {
                for (idx, stmt) in body.iter().enumerate() {
                    path.push(format!("block_stmt[{idx}]"));
                    walk_stmt(stmt, path);
                    path.pop();
                }
            }
            Expression::Function { body, .. } => match body {
                tricti::ast::FunctionBody::Expression(expr) => {
                    path.push("fn_expr".into());
                    walk_expr(expr, path);
                    path.pop();
                }
                tricti::ast::FunctionBody::Block(stmts) => {
                    for (idx, stmt) in stmts.iter().enumerate() {
                        path.push(format!("fn_stmt[{idx}]"));
                        walk_stmt(stmt, path);
                        path.pop();
                    }
                }
            },
            Expression::Tuple(elements) => {
                for (idx, elem) in elements.iter().enumerate() {
                    path.push(format!("tuple[{idx}]"));
                    walk_expr(elem, path);
                    path.pop();
                }
            }
            Expression::Match { value, arms } => {
                path.push("match_value".into());
                walk_expr(value, path);
                path.pop();
                for (idx, arm) in arms.iter().enumerate() {
                    path.push(format!("match_arm[{idx}].pattern"));
                    walk_expr(&arm.pattern, path);
                    path.pop();
                    path.push(format!("match_arm[{idx}].body"));
                    walk_expr(&arm.body, path);
                    path.pop();
                }
            }
            Expression::StructLiteral { fields, .. } => {
                for (name, expr) in fields {
                    path.push(format!("struct_field({name})"));
                    walk_expr(expr, path);
                    path.pop();
                }
            }
            Expression::VecNew {
                length,
                fill,
                additional_dimensions,
                ..
            } => {
                if let Some(len_expr) = length {
                    path.push("vec_length".into());
                    walk_expr(len_expr, path);
                    path.pop();
                }
                if let Some(fill_expr) = fill {
                    path.push("vec_fill".into());
                    walk_expr(fill_expr, path);
                    path.pop();
                }
                for (idx, dim) in additional_dimensions.iter().enumerate() {
                    path.push(format!("vec_dim[{idx}]"));
                    walk_expr(dim, path);
                    path.pop();
                }
            }
            Expression::VecLiteral { elements } => {
                for (idx, elem) in elements.iter().enumerate() {
                    path.push(format!("vec_literal[{idx}]"));
                    walk_expr(elem, path);
                    path.pop();
                }
            }
            Expression::Matrix { rows } => {
                for (r_idx, row) in rows.iter().enumerate() {
                    for (c_idx, cell) in row.iter().enumerate() {
                        path.push(format!("matrix[{r_idx}][{c_idx}]"));
                        walk_expr(cell, path);
                        path.pop();
                    }
                }
            }
            Expression::Range { start, end, step } => {
                path.push("range_start".into());
                walk_expr(start, path);
                path.pop();
                path.push("range_end".into());
                walk_expr(end, path);
                path.pop();
                if let Some(step) = step {
                    path.push("range_step".into());
                    walk_expr(step, path);
                    path.pop();
                }
            }
            Expression::Question(inner) | Expression::Unwrap(inner) => {
                path.push("question_inner".into());
                walk_expr(inner, path);
                path.pop();
            }
            Expression::Literal(_)
            | Expression::Identifier(_)
            | Expression::StaticPath { .. }
            | Expression::Query(_)
            | Expression::Shader { .. } => {}
        }
    }

    fn walk_stmt(stmt: &Statement, path: &mut Vec<String>) {
        match stmt {
            Statement::VariableDecl { value, .. }
            | Statement::Assignment { value, .. }
            | Statement::Expression(value)
            | Statement::Return(Some(value))
            | Statement::Break(Some(value)) => {
                path.push("stmt_expr".into());
                walk_expr(value, path);
                path.pop();
            }
            Statement::ConstDecl { value, .. } => {
                path.push("const_value".into());
                if let tricti::ast::ConstValue::Expression(expr) = value {
                    walk_expr(expr, path);
                }
                path.pop();
            }
            Statement::ForLoop { iterable, body, .. } => {
                path.push("for_iterable".into());
                walk_expr(iterable, path);
                path.pop();
                for (idx, stmt) in body.iter().enumerate() {
                    path.push(format!("for_body[{idx}]"));
                    walk_stmt(stmt, path);
                    path.pop();
                }
            }
            Statement::Return(None)
            | Statement::Break(None)
            | Statement::Continue
            | Statement::Use { .. }
            | Statement::ImplBlock { .. }
            | Statement::ImplMethod { .. }
            | Statement::ModuleDecl { .. }
            | Statement::IfDef { .. } => {}
        }
    }

    let mut path = Vec::new();
    for (idx, stmt) in program.statements.iter().enumerate() {
        path.push(format!("program[{idx}]"));
        walk_stmt(stmt, &mut path);
        path.pop();
    }
}

fn clang_available() -> bool {
    Command::new("clang").arg("--version").output().is_ok()
}

#[test]
fn assert_fails_exits_nonzero() {
    if !clang_available() {
        eprintln!("clang not found; skipping");
        return;
    }
    let prelude = fs::read_to_string("stdlib/prelude.tri").expect("read prelude");
    let user = r#"
        main :: () => {
            assert(false, "boom");
            println(999); # unreachable
        }
    "#;
    let src = format!("{}\n{}", prelude, user);

    let program = parser::parse(src.to_string());
    log_if_identifiers(&program);
    let sem = semantic::analyze_program(&program).expect("semantic analysis");

    let context = Context::create();
    let mut gen = codegen::CodeGenerator::new(&context, sem).expect("codegen ctx");
    gen.generate_program(&program).expect("codegen");

    let obj = "tests/tmp_assert_fail.o";
    let exe = "tests/tmp_assert_fail.out";
    if Path::new(obj).exists() {
        let _ = fs::remove_file(obj);
    }
    if Path::new(exe).exists() {
        let _ = fs::remove_file(exe);
    }
    gen.write_object_file(obj).expect("write obj");

    let status = Command::new("clang")
        .args(["-no-pie", "-o", exe, obj])
        .status()
        .expect("link");
    assert!(status.success(), "link failed");

    let out = Command::new(exe).output().expect("run");
    assert!(!out.status.success(), "program should have exited non-zero");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout, "Assertion failed: boom\n");
}
