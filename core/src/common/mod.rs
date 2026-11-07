//! 纯算法层：输入分流（混合识别）、帮助解析、渲染辅助、哈希、脚本装配。
//! 全部为纯函数（无 IO、无副作用），可独立单测。

pub mod hash;
pub mod help_parser;
pub mod marker_parser;
pub mod render;
pub mod script_assembler;
pub mod shell_split;
