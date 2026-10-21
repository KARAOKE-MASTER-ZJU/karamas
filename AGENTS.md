# 仓库开发规范

## 工程结构

- `src/` 存放 TypeScript 前端代码。
- `src-tauri/` 负责 Tauri 初始化、命令桥接和平台接入。
- `crates/casting-core/` 存放独立于应用框架的 Rust 投屏核心。
- 根目录通过 Cargo workspace 统一管理 Rust crate，共用依赖锁文件和构建目录。

## 开发约定

- Tauri 命令层只负责调用转发，投屏逻辑放在核心库中。
- 核心库保持独立，不依赖 Tauri、浏览器类型或前端状态。
- 有具体功能需要时，再添加模块和依赖。
- TypeScript 使用严格模式，前端代码使用两个空格缩进，新增 Rust 代码使用 rustfmt 格式化。
- 保留迁入核心代码的现有格式，按任务需要修改相关代码。
- 修改工程骨架后，运行 `pnpm check`、`pnpm build` 和 `cargo check --workspace`。
- 修改核心代码或 Cargo 集成配置后，运行 `cargo test -p ktv-casting --lib`。
- 文档、说明和面向用户的文字使用中文，命令、路径和技术名称保留原文。
- README.md 面向使用者介绍项目和运行方式；源码迁入、兼容性、工程实现和后续接入说明放在 DEVELOP.md。

## 授权边界

本地修改和检查不代表获得提交、改写提交历史、推送、创建 PR 或发布的授权。执行这些操作前，必须获得用户明确授权。
