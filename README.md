# RustSSH

Windows 上的桌面 SSH 客户端。用 Rust 和 [iced](https://iced.rs) 写成，带会话管理、终端和 SFTP 文件浏览。

## 功能

- 快速连接，地址写成 `user@host`、`user@host:port`、`host:port` 或 `ssh://user@host:port`
- 保存会话，按分组浏览；把会话拖到另一个分组标题上可以改分类
- 多标签终端。拖拽选择文字复制，右键粘贴，`Ctrl+Tab` 切换标签
- 查看滚动日志时按 Enter，画面回到最新一行；`less`、`vim` 这类全屏程序不会被拽回底部
- 左侧终端图标打开或收起会话栏，文件夹图标打开或收起 SFTP
- 主机密钥第一次连接时确认，之后记在本机；指纹变化会提示

## 环境

- Windows
- [Rust](https://rustup.rs) stable（edition 2021）

加密使用 `ring`，不依赖 `aws-lc-rs`。

## 运行

```powershell
cargo run
```

调试构建会编译依赖，第一次会比较久。测试：

```powershell
cargo test
```

## 数据

配置写在用户配置目录下的 `rustssh` 文件夹。Windows 上一般是：

```text
%APPDATA%\rustssh\
  sessions.json       保存的会话
  known_hosts.json    已确认的主机密钥指纹
```

勾选记住密码时，密码以明文写进 `sessions.json`。这只适合本机自己用，不要把这个文件提交到仓库或拷给别人。

## 代码结构

| 路径 | 内容 |
| --- | --- |
| `src/app/` | 界面：状态、消息、会话栏、终端绘制、SFTP 面板 |
| `src/ssh/` | 连接、终端通道、SFTP 传输、远程监控 |
| `src/term.rs` | 终端模拟：屏幕、ANSI、按键 |
| `src/model.rs` | 会话和已知主机的读写、地址解析 |

## 许可

MIT
