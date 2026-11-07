//! POSIX shell 词切分（纯函数）：账本 argv 与 `sh -c` 真实执行语义对齐。
//! infra 以 `sh -c <command>` 执行，空白切分会把 `echo "a b"` 记成三个 argv——
//! 此处按引号/转义切分还原真实参数边界；引号不闭合（sh 会报错）退空白切分兜底。

/// 引号感知切分：单引号字面、双引号内 `\"` `\\` `\$` `` \` `` 转义、引号外 `\x` 转义。
/// 解析失败（引号不闭合/尾悬反斜杠）→ Err，调用方退回空白切分。私有：公开面只留 argv_of。
fn split_command(cmd: &str) -> Result<Vec<String>, ()> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = cmd.chars().peekable();
    let mut started = false;
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return Err(()),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            // 双引号内仅 \" \\ \$ \` 有 shell 转义语义，其余保字面
                            Some(e @ ('"' | '\\' | '$' | '`')) => cur.push(e),
                            Some(e) => {
                                cur.push('\\');
                                cur.push(e);
                            }
                            None => return Err(()),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err(()),
                    }
                }
            }
            '\\' => match chars.next() {
                Some(e) => {
                    started = true;
                    cur.push(e);
                }
                None => return Err(()),
            },
            c if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                started = true;
                cur.push(c);
            }
        }
    }
    if started {
        out.push(cur);
    }
    Ok(out)
}

/// 切分（永不失败）：引号不闭合退空白切分（与旧行为一致，不产生更差结果）。
pub fn argv_of(command: &str) -> Vec<String> {
    split_command(command)
        .unwrap_or_else(|_| command.split_whitespace().map(str::to_string).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_preserve_argument_boundaries() {
        assert_eq!(
            argv_of("echo \"a b\""),
            vec!["echo".to_string(), "a b".to_string()],
            "双引号参数不得被空白拆散"
        );
        assert_eq!(
            argv_of("touch 'my file.txt'"),
            vec!["touch".to_string(), "my file.txt".to_string()]
        );
    }

    #[test]
    fn escapes_and_plain_words() {
        assert_eq!(argv_of(r"a\ b c"), vec!["a b".to_string(), "c".to_string()]);
        assert_eq!(
            argv_of("'it''s'"),
            vec!["its".to_string()],
            "相邻引号段拼接"
        );
        assert_eq!(
            argv_of("rm -rf ./build"),
            vec!["rm".to_string(), "-rf".to_string(), "./build".to_string()]
        );
        assert_eq!(argv_of(""), Vec::<String>::new());
        assert_eq!(argv_of("   "), Vec::<String>::new());
    }

    #[test]
    fn double_quote_semantic_escapes() {
        assert_eq!(
            argv_of(r#"echo "\$"x"#),
            vec!["echo".to_string(), "$x".to_string()],
            "双引号内 \\$ 转义为字面 $"
        );
        assert_eq!(
            argv_of(r#""a\nb""#),
            vec![r"a\nb".to_string()],
            "双引号内无 shell 语义的转义保字面"
        );
    }

    #[test]
    fn unterminated_quote_falls_back_to_whitespace() {
        assert_eq!(
            argv_of("echo \"unclosed"),
            vec!["echo".to_string(), "\"unclosed".to_string()]
        );
        assert_eq!(argv_of("a\\"), vec!["a\\".to_string()]);
    }
}
