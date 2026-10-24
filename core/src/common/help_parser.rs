//! I-002：help 文本 → 参数 schema 解析（algorithm，纯函数）。
//! 覆盖 GNU 风格（ls --help）与 Cobra 风格（子命令段）；解析失败=false（evolution 说明）。

use crate::framework::{Context, Evolution, Metrics};
use crate::knowledge::{CommandKnowledge, ParamSpec};
use crate::types::Platform;

/// 解析输入：命令名 + 平台 + help 原文。
#[derive(Debug, Clone, Default)]
pub struct HelpText {
    pub command: String,
    pub platform: Platform,
    pub text: String,
}

/// I-002：解析 help 文本。输出经 source_hash 指纹锚定；格式不识别返回 false。
pub fn parse_help_text(
    input: &HelpText,
    output: &mut CommandKnowledge,
    _context: &Context,
    metrics: &mut Metrics,
    evolution: &mut Evolution,
) -> bool {
    let params = extract_params(&input.text);
    let subcommands = extract_subcommands(&input.text);
    if params.is_empty() && subcommands.is_empty() {
        evolution.fail("IAC-1104", "help 文本未识别出参数或子命令");
        return false;
    }
    *output = CommandKnowledge {
        command: input.command.clone(),
        platform: input.platform,
        subcommands,
        params,
        source_hash: crate::common::hash::fnv1a64_hex(&input.text),
        verified: true,
    };
    metrics.counter("knowledge.parsed", 1);
    true
}

/// 提取选项行：`  -a, --all [VALUE]  说明文字` → ParamSpec。
fn extract_params(text: &str) -> Vec<ParamSpec> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('-') || trimmed.len() < 2 {
            continue;
        }
        let Some((flag_part, meaning)) = split_flag_line(trimmed) else { continue };
        for flag in extract_flags(&flag_part) {
            out.push(ParamSpec { flag, meaning: meaning.trim().to_string(), required: false, default: None });
        }
    }
    out
}

/// 拆分选项行：以 2+ 空格为界取 (旗标段, 说明段)。
fn split_flag_line(line: &str) -> Option<(String, String)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() - 1 {
        if bytes[i] == b' ' && bytes[i + 1] == b' ' {
            let flag_part = line[..i].trim().to_string();
            let meaning = line[i..].trim().to_string();
            if !flag_part.starts_with('-') {
                return None;
            }
            return Some((flag_part, meaning));
        }
        i += 1;
    }
    None
}

/// 从旗标段提取 flag token（-a / --all / --level <N> 中的 --level）。
fn extract_flags(flag_part: &str) -> Vec<String> {
    flag_part
        .split_whitespace()
        .filter(|t| t.starts_with('-'))
        .map(|t| t.split('<').next().unwrap_or(t).split('=').next().unwrap_or(t).trim_end_matches(',').to_string())
        .collect()
}

/// 提取子命令段：`Commands:`/`Available Commands:` 之后缩进行的第一个词。
fn extract_subcommands(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        let lower = line.trim().to_lowercase();
        if lower.starts_with("commands:") || lower.starts_with("available commands:") {
            in_section = true;
            continue;
        }
        if in_section {
            if line.trim().is_empty() || (!line.starts_with(' ') && !line.starts_with('\t')) {
                in_section = false;
                continue;
            }
            if let Some(name) = line.split_whitespace().next() {
                if !name.starts_with('-') {
                    out.push(name.to_string());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Option<CommandKnowledge> {
        let env = crate::framework::TestEnv::new();
        let input = HelpText { command: "ls".into(), platform: Platform::Linux, text: text.into() };
        let mut out = CommandKnowledge {
            command: String::new(),
            platform: Platform::Linux,
            subcommands: vec![],
            params: vec![],
            source_hash: String::new(),
            verified: false,
        };
        let mut m = Metrics::default();
        let mut e = Evolution::default();
        parse_help_text(&input, &mut out, &env.context(), &mut m, &mut e).then_some(out)
    }

    const GNU_HELP: &str = "Usage: ls [OPTION]... [FILE]...\n\
List information about the FILEs.\n\
\n\
  -a, --all                  do not ignore entries starting with .\n\
  -l                         use a long listing format\n\
  -h, --human-readable       with -l, print human readable sizes\n\
      --block-size=SIZE      scale sizes by SIZE\n";

    const COBRA_HELP: &str = "Usage:\n  docker [command]\n\nAvailable Commands:\n  run     Run a command in a container\n  ps      List containers\n\nFlags:\n  -H, --host string     Daemon socket\n  -h, --help            help for docker\n";

    #[test]
    fn parses_gnu_style_options() {
        let k = parse(GNU_HELP).expect("GNU help 应可解析");
        assert!(k.verified);
        assert!(k.params.iter().any(|p| p.flag == "--all" && p.meaning.contains("ignore")));
        assert!(k.params.iter().any(|p| p.flag == "-l"));
        assert!(k.params.iter().any(|p| p.flag == "--block-size"));
        assert!(k.subcommands.is_empty());
    }

    #[test]
    fn parses_cobra_style_subcommands() {
        let k = parse(COBRA_HELP).expect("Cobra help 应可解析");
        assert_eq!(k.subcommands, vec!["run", "ps"]);
        assert!(k.params.iter().any(|p| p.flag == "--host"));
    }

    #[test]
    fn unrecognized_text_fails_with_1104() {
        let out = parse("这不是 help 文本，没有任何参数行");
        assert!(out.is_none());
    }
}
