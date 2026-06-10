# PKULaborTheoryAutoLearn

一个最小化的 Python 脚本，用来：

- 调用 `app/info` 获取应用配置
- 走 PKU IAAA 登录，拿到 `token_code`
- 把 `token_code` 换成 Readoor Bearer token
- 调用 `sections` 接口列出课程小节
- 构造并发送 `pStatIf` 请求

## 依赖

先安装：

```bash
pip install requests pycryptodome
```

## 最常用命令

只测试登录，打印最终 Bearer token：

```bash
python readoor_pstatif_probe.py --username "你的学号" --password "你的密码" --login-only
```

列出当前课程的 sections：

```bash
python readoor_pstatif_probe.py --token "你的BearerToken" --module-id 12097 --list-sections --dump-only
```

交互式选择 section，并发送 `pStatIf`：

```bash
python readoor_pstatif_probe.py --token "你的BearerToken" --module-id 12097 --choose-section
```

直接指定某个 section：

```bash
python readoor_pstatif_probe.py --token "你的BearerToken" --module-id 12097 --section-guid 570542196487401472
```

也可以把登录和发送合并：

```bash
python readoor_pstatif_probe.py --username "你的学号" --password "你的密码" --module-id 12097 --choose-section
```

## 常用参数

- `--username` / `--password`：走 PKU IAAA 登录
- `--token`：直接使用现成 Bearer token
- `--token-code`：跳过 IAAA，直接做 `token_code -> token`
- `--module-id`：调用 `sections` 时需要
- `--section-guid`：指定 section
- `--choose-section`：交互式选择 section
- `--list-sections`：打印 sections 摘要
- `--dump-only`：只打印请求数据，不真正发送
- `--login-only`：只做登录/换 token

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
