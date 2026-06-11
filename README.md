# PKULaborTheoryAutoLearn

当前仓库已经提供 Rust 命令行工具 `auto_learn`，用于：

- 调用 `app/info` 获取应用配置
- 走 PKU IAAA 登录，拿到 `token_code`
- 把 `token_code` 换成 Readoor Bearer token
- 调用 `sections` 接口列出课程小节
- 构造并发送 `pStatIf` 请求

旧版 Python 原型脚本仍保留在 `readoor_pstatif_probe.py` 里，方便对照；日常使用建议直接用 Rust 二进制。

## 构建

需要本机安装 Rust 工具链，然后执行：

```bash
cargo build --release
```

构建完成后，单个可执行文件位于：

```bash
target/release/auto_learn.exe
```

## 最常用命令

只测试登录，打印最终 Bearer token：

```bash
auto_learn --username "你的学号" --password "你的密码" --login-only
```

列出当前课程的 sections：

```bash
auto_learn --token "你的BearerToken" --module-id 12097 --list-sections --dump-only
```

交互式选择 section，并发送 `pStatIf`：

```bash
auto_learn --token "你的BearerToken" --module-id 12097 --choose-section
```

交互输入时支持逗号分隔多选，例如：

```text
1,2,3,4
```

直接指定某个 section：

```bash
auto_learn --token "你的BearerToken" --module-id 12097 --section-guid 570542196487401472
```

也支持一次指定多个 section：

```bash
auto_learn --token "你的BearerToken" --module-id 12097 --section-guid 570542196487401472,570542212652249088
```

也可以把登录和发送合并：

```bash
auto_learn --username "你的学号" --password "你的密码" --module-id 12097 --choose-section
```

## 常用参数

- `--username` / `--password`：走 PKU IAAA 登录
- `--token`：直接使用现成 Bearer token
- `--token-code`：跳过 IAAA，直接做 `token_code -> token`
- `--iaaa-oauth-url`：手动指定完整 IAAA 登录 URL。通常不需要；脚本会先访问 Readoor 的北大登录入口，动态拿一份新的 `oauth.jsp` URL
- `--module-id`：调用 `sections` 时需要
- `--section-guid`：指定一个或多个 section，支持逗号分隔
- `--choose-section`：交互式选择一个或多个 section，支持逗号分隔
- `--list-sections`：打印 sections 摘要
- `--dump-only`：只打印请求数据，不真正发送
- `--login-only`：只做登录/换 token
- `--debug`：显示 IAAA 回调、token 交换、HTTP 响应和请求载荷等调试输出

## 说明

- 当前脚本默认 app 是 `550278742975483904`
- 当前脚本已经不依赖复制浏览器 cookies
- `app_id`、`company_guid`、`idaas_id` 会通过 `app/info` 自动获取
- `class_id`、`train_id`、`project_id`、`task_guid` 仍有一部分来自当前样本，脚本还没有完全泛化到任意课程

## 已知问题

- 如果 token 过期或被拉黑，接口会返回 `401`，例如：

```json
{"status":-1,"code":401,"message":"Token has been blacklisted","err_code":"err_idas_sign_0008"}
```

- 遇到这种情况，重新登录拿一份新 token 即可再试
