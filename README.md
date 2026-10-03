# sysenv

跨平台（Windows / Ubuntu）**系统 PATH 与环境变量管理工具**，单一二进制、子命令划分功能。内置六大能力：



* **PATH 管理**：增删查改系统 PATH，自动持久化并即时作用于当前环境

* **注册表导入导出**：PATH 一键导出为注册表 / JSON / 文本，可反向合并或整体替换

* **链接到 PATH**：把任意文件挂进 PATH 目录，之后任意目录直接敲名字即可执行

* **环境变量管理**：读写系统环境变量，可选择持久化或仅临时生效

* **AI 模型查询**：从 models.dev 按名称爬取模型 / Provider 信息

* **httpie 兼容 HTTP 客户端**：参数与 httpie 保持一致

* **快捷命令垫片**：一键生成 `spath / senv / slink / shttp / sai` 短命令



```
sysenv 0.2.1

Usage: sysenv <COMMAND>

Commands:
  path   Manage PATH entries: list / add / remove / has / export / import
  env    Read / set / unset / list environment variables
  link   Link a file into a PATH directory so it runs from anywhere
  http   httpie-compatible HTTP client: http [flags] [METHOD] URL [ITEM...]
  ai     Query the models.dev database of AI models & providers
  short  Install shell shims for every subcommand (spath/senv/slink/shttp/sai)
```



***

## 特性总览



| 功能        | 子命令                  | 核心特性                                                                                   |
| --------- | -------------------- | -------------------------------------------------------------------------------------- |
| PATH 增删查改 | `path`               | 持久化 + 当前会话即时生效；自动转绝对路径与去重；前置插入；user/machine 双作用域；`--temporary` 临时模式                    |
| 注册表导入导出   | `path export/import` | `.reg` / JSON / TXT 三种格式；合并或 `--replace` 整体替换；跨机迁移备份                                   |
| 链接到 PATH  | `link`               | 硬链接 → 符号链接 → 拷贝三级自动回退；Windows `.cmd` 垫片；自定义命令名；系统目录或托管目录                               |
| 环境变量      | `env`                | get /set/unset /list；默认持久化；`--temporary` 仅当前 shell；machine 作用域                         |
| AI 模型查询   | `ai`                 | 226 个 Provider、8000+ 模型；24h 本地缓存；canonical 优选；`--search / --list / --json / --refresh` |
| HTTP 客户端  | `http`               | httpie 参数子集对齐；JSON / 表单 /multipart/ 原始体；嵌套 JSON；下载 / 重定向 / 认证 / 离线模式                   |
| 快捷垫片      | `short`              | 一键安装五种短命令；Windows `.cmd` / Linux sh 脚本；自动加入 PATH                                       |



***

## 构建

需要 Rust 1.79+（`rustup` / `cargo`）。



```
cargo build --release
# 产物: target/release/sysenv(.exe)
```

### 发布产物

* 发布产物统一 **UPX 压缩** 后放入 `dist/` 目录

* 文件名包含平台 + 架构 + 版本号：`sysenv-<平台>-<架构>_v<版本>`

  * Windows：`sysenv-windows-x86_64_v0.2.1.exe`

  * Ubuntu：`sysenv-linux-x86_64_v0.2.1`

* 压缩命令（示例，Windows）：

  ```
  upx --best -o dist/sysenv-windows-x86_64_v0.2.1.exe target/release/sysenv.exe
  ```

* 版本号变更与新增 / 修复内容记录在 `patch.md`

## 平台与持久化机制



* **Windows**：默认写用户作用域注册表 `HKCU\Environment`（新进程自动继承）；`--scope machine` 写 `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`，需管理员。

* **Ubuntu**：默认写入 `~/.config/sysenv/` 托管文件，并自动在 shell 启动脚本中注入，使持久化变量 / 路径对新 shell 生效；machine 作用域写 `/etc/environment`，需 root。

* 无论哪种平台，`path add`、`env set` 都会**同时更新当前进程环境**（Linux 下通过向父 shell 输出可粘贴的 export 片段，Windows 下通过注册表广播），保证 "持久化 + 当前生效" 双满足。

* 所有写操作默认持久化；加 `--temporary` 则只打印可粘贴的 shell 片段（`export` / `$env:`），不写入任何持久化存储。



***

## 1. PATH 管理（`path`）

### 特性



* `list`：列出当前生效的完整 PATH（合并了进程与持久化作用域）

* `add`：追加到末尾；`-p/--prepend` 插入到开头；自动转绝对路径、大小写不敏感去重、跳过空条目

* `remove`：按绝对路径匹配移除（任意作用域）

* `has`：同时检查当前进程与已持久化作用域，报告该目录是否在 PATH 中

* `export / import`：注册表文件（`.reg`）、JSON（`.json`）、纯文本（`.txt`）三种格式双向转换

* `--scope user|machine`：控制写入作用域

* `--temporary`：不持久化，仅打印可粘贴的 PATH 片段

### 示例



```
sysenv path list                      # 列出当前生效 PATH
sysenv path add D:\tools              # 追加（自动转绝对路径、去重）
sysenv path add D:\tools -p           # 前置
sysenv path add /opt/bin --scope machine   # 机器级（需管理员/root）
sysenv path add D:\tools --temporary # 仅当前会话（打印可粘贴片段）
sysenv path remove D:\tools
sysenv path has D:\tools              # 同时检查当前进程与已持久化作用域
```

## 2. PATH 导入 / 导出（`path export/import`）

### 特性



* `export` 按目标扩展名自动选择格式：`.reg`（Windows Registry Editor 格式，可双击导入）、`.json`（结构化，含作用域信息）、`.txt`（一行一个条目）

* `import` 默认**合并**（按绝对路径去重后追加）；`--replace` 整体替换

* 适合跨机器迁移、备份还原、批量修改前快照

### 示例



```
sysenv path export backup.reg         # Windows Registry Editor 格式，可双击导入
sysenv path export paths.json
sysenv path import backup.reg         # 合并进用户 PATH（去重）
sysenv path import backup.reg --replace   # 整体替换
sysenv path export paths.txt          # Ubuntu 下交换备份
```

## 3. 链接到 PATH（`link`）

### 特性



* 把任意路径下的文件放进一个 **PATH 目录**，之后任意目录直接敲名字执行

* 链接方式自动尝试：**硬链接 → 符号链接 → 拷贝**（跨卷 / 无权限时逐级自动回退），Windows 下需要开发者模式时自动降级拷贝

* Windows 对非 `.exe` 目标（`.py` / `.cmd` / 脚本等）自动生成同名 `.cmd` 垫片，保证 cmd / PowerShell 都能直接调用

* `--name` 自定义命令名；`--method hard|symlink|copy` 强制指定方式

* `--dir` 指定安装目录；`--system` 使用系统目录（Windows `System32` / Linux `/usr/local/bin`）

* 托管目录默认：Windows `%USERPROFILE%\.sysenv\bin`，Ubuntu `~/.local/bin`；目录不在 PATH 时自动持久化加入（`--temporary` 则只打印片段）

### 示例



```
sysenv link mytool.exe                # 放入托管 bin 目录并自动加入 PATH
sysenv link ./script.py               # Windows 自动生成 script.cmd 垫片
sysenv link tool --name t             # 改名
sysenv link tool --method copy        # 强制用拷贝（跨盘时自动回退）
sysenv link tool --system             # 放到系统目录
sysenv link tool -f                   # 覆盖已存在的链接
```

## 4. 环境变量（`env`）

### 特性



* `get NAME`：读取当前进程生效值（含继承链）

* `set NAME VALUE`：默认**持久化**并同时更新当前进程；`--temporary` 仅打印 `export` / `$env:` 片段；`--scope machine` 写机器级

* `unset NAME`：删除持久化定义，并同步从当前进程移除

* `list`：列出全部已持久化的变量（键值对），支持 `--format` 输出为 env 文件 / JSON / 纯文本（与导入格式互通）

* 变量值包含空格、特殊字符时自动做 shell 转义，粘贴片段即可直接使用

### 示例



```
sysenv env get FOO
sysenv env set FOO bar                # 持久化（默认）
sysenv env set FOO bar --temporary    # 仅当前 shell（打印 export / $env: 片段）
sysenv env set FOO bar --scope machine
sysenv env unset FOO
sysenv env list
```

## 5. AI 模型 / Provider 查询（`ai`）

### 特性



* 数据源为 [models.dev](https://models.dev) 官方公开数据 `https://models.dev/api.json`（约 5.3 MB，**226 个 Provider、8000+ 模型**）；`/models/` 与 `/providers/` 页面即由此数据渲染

* **24 小时本地缓存**：Windows `%LOCALAPPDATA%\sysenv\`、Linux `$XDG_CACHE_HOME` 或 `~/.cache/sysenv/`；`--refresh` 强制重新抓取

* 查询匹配：大小写不敏感，精确匹配模型 `id` / `canonical_model_id` / `name`，未命中自动降级为子串

* **canonical 优选**：多个 Provider 暴露同名模型时，按 `canonical_model_id` 前缀多数投票选出主条目（如 `gpt-4.1` → OpenAI），打印详情并提示其余 Provider 数量；也可用完整 id（`openai/gpt-4.1-mini`）精确定位

* `-s/--search`：grep 风格列表输出（`id 名称 provider` 每行一条），`--limit` 控制条数（默认 20）

* `--list`：分页浏览全部模型 / Provider

* `-o/--output-format json|csv`：机器可读输出——`json` 输出 **JSON 数组**（与 `--json` 等价，单命中也是数组）；`csv` 输出带表头的 CSV 表格（model 24 列、provider 6 列，RFC-4180 转义），适用于详情、搜索、列表与多匹配全部场景

* 模型详情字段：`id / name / provider / family / description / modalities / context / output limit / cost（每 1M tokens 的 input/output/cache_read 折算美元）/ reasoning / tool call / structured output / temperature / attachment / open weights / release date / last updated / knowledge cutoff / reasoning options` 等

### 示例



```
sysenv ai model gpt-4.1                  # 精确查询；多 provider 同名时自动选 canonical 并提示其余
sysenv ai model openai/gpt-4.1-mini      # 用完整 id 精确定位
sysenv ai model -s qwen --limit 10       # 子串搜索（grep 风格列表）
sysenv ai model --list --limit 5         # 列出模型（共 8000+）
sysenv ai model gpt-4.1 --json           # 输出 JSON 数组（同 -o json）
sysenv ai model gpt-4.1 -o csv           # 输出 CSV 表格（24 列）
sysenv ai model -s qwen -o csv           # 搜索结果的 CSV 输出
sysenv ai provider openai -o csv         # Provider CSV（6 列）
sysenv ai model gpt-4.1 --refresh        # 强制重抓数据
sysenv ai provider openai                # Provider 详情（api / npm / models 数量与清单）
sysenv ai provider -s groq               # Provider 子串搜索
sysenv ai provider --list                # 列出全部 Provider
```

CSV 列：model 为 `id,name,provider,family,status,knowledge_cutoff,description,context,input_limit,output_limit,cost_input,cost_output,cost_cache_read,modalities_input,modalities_output,reasoning,tool_call,structured_output,temperature,attachment,open_weights,release_date,last_updated,canonical_model_id`；provider 为 `id,name,api,env,npm,models_count`。

## 6. httpie 兼容 HTTP 客户端（`http`）

参数与 [httpie](https://httpie.io) 保持一致（子集）。

### 特性



* 方法自动推导：URL 项中不含方法时自动 GET；含数据项时自动 POST

* 默认 JSON：`key=value` 自动构造成 JSON 对象；`-f/--form` 切换表单、`--multipart` 文件上传

* 完整请求项语法（与 httpie 一致）：数据字段、原始 JSON、查询参数、请求头、multipart 文件、`@file` 原始体、嵌套 JSON 构建（`a[b][c]=v`、`a[]=v`、`a[1]=v`）

* 输出控制：`-p/--print`（`BHbh` 任意组合）、`-h` 仅头、`-b` 仅体、`-m` 状态行、`-v` 全量；终端下默认 `hb`，管道 / 重定向下自动仅 `b`

* `-o/--output` 响应体存文件（其余信息打 stderr）、`-d/--download` wget 式下载

* 认证：`-a user:pass` Basic、`-A bearer -a TOKEN` Bearer

* 网络行为：`-F/--follow` 跟随重定向、`--max-redirects`、`--timeout`、`--proxy`、`--verify no` 跳过证书校验、`--offline` 只构建并打印请求不发送、`-I/--ignore-stdin`

* `--check-status`：3xx → 退出码 3，4xx → 4，5xx → 5（脚本友好）

* stdin 管道直接作为原始请求体；`--raw` 显式指定原始体

* 终端美化：默认对 JSON 响应做缩进格式化，`--pretty none` 关闭

### 示例



```
sysenv http pie.dev/get                       # GET（终端默认显示状态行+头+体）
sysenv http pie.dev/post name=John age:=29    # 无方法时自动 POST；默认 JSON
sysenv http -f POST pie.dev/post name='John Smith'   # 表单
sysenv http -v pie.dev/get                    # 打印完整请求与响应
sysenv http -h pie.dev/get                    # 只打印响应头
sysenv http GET pie.dev/get q==httpie per_page==1   # 查询参数
sysenv http pie.dev/post X-API-Token:123 name=John  # 请求头 + JSON 字段
sysenv http -d pie.dev/image.png              # wget 式下载
sysenv http -o out.json pie.dev/get           # 响应体存文件（其余打到 stderr）
sysenv http POST pie.dev/post @data.json      # 文件作为原始请求体
sysenv http pie.dev/post cv@resume.pdf        # multipart 文件上传
sysenv http -a user:pass pie.dev/anything     # Basic Auth
sysenv http -A bearer -a TOKEN pie.dev/anything   # Bearer Token
sysenv http -F --max-redirects 5 pie.dev/     # 跟随重定向
sysenv http --check-status pie.dev/404        # 退出码 = 4
sysenv http --offline pie.dev/post a=1        # 只构建并打印请求，不发送
sysenv http POST pie.dev/post --raw '{"a":1}' # 显式原始体
sysenv http pie.dev/post -- -name=foo         # 以 - 开头的字段名需跟在 -- 之后
echo '{"a":1}' | sysenv http POST pie.dev/post  # stdin 作为原始体
sysenv http --verify no https://self-signed.example  # 跳过证书校验
```

> PowerShell 注意：PS 5.1 会剥掉传给原生程序参数中的内嵌双引号，含引号的 JSON / 原始体请用 
>
> `\"`
>
>  转义、单引号包裹或在 cmd/bash 下执行。

### 请求项语法



| 项                                                       | 含义                                   |
| ------------------------------------------------------- | ------------------------------------ |
| `key=value`                                             | 数据字段（默认 JSON，`-f` 时为表单）              |
| `key:=json`                                             | 原始 JSON 值（数字 / 布尔 / 对象 / 数组）         |
| `key==value`                                            | URL 查询参数                             |
| `key:value`                                             | 请求头（`key:` 空值 = 取消默认头，`key;` = 发送空头） |
| `key@file`                                              | multipart 文件上传（`;type=mime` 可指定类型）   |
| `key=@file` / `key:=@file` / `key==@file` / `key:@file` | 从文件读取字段 / JSON / 查询 / 头的值            |
| `@file`                                                 | 原始请求体（也可用 `--raw` 或管道 stdin）         |
| `a[b][c]=v`、`a[]=v`、`a[1]=v`、`[]:=1`                    | 嵌套 JSON 构建                           |

### 支持的参数速查

`-j/--json` `-f/--form` `--multipart` `--raw` `-p/--print` `-h/--headers` `-b/--body`

`-m/--meta` `-v/--verbose` `-o/--output` `-d/--download` `-q/--quiet` `--pretty`

`-a/--auth` `-A/--auth-type` `--proxy` `-F/--follow` `--max-redirects` `--timeout`

`--check-status` `--offline` `--verify` `-I/--ignore-stdin` `--default-scheme`

> 说明：http 子命令中 
>
> `-h`
>
>  是 httpie 语义的 "只打印响应头"，因此帮助请用 
>
> `sysenv help http`
>
> 。

## 7. 快捷命令垫片（`short`）

### 特性



* 一键为**全部子命令**生成短命令垫片，之后任意目录直接使用

* Windows 生成 `.cmd` 垫片（`@echo off` + 转发），Ubuntu 生成可执行 `sh` 脚本（`chmod 755`），均转发全部参数

* 默认安装到托管 bin 目录（与 `link` 共用）并**自动持久化加入 PATH**；`--temporary` 只打印片段不写入

* `--dir` 指定目录、`-f/--force` 覆盖已存在的垫片

* 垫片转发 `%*` / `"$@"`，无参数数量限制，特殊字符安全



| 短命令     | 等价            |
| ------- | ------------- |
| `spath` | `sysenv path` |
| `senv`  | `sysenv env`  |
| `slink` | `sysenv link` |
| `shttp` | `sysenv http` |
| `sai`   | `sysenv ai`   |

### 示例



```
sysenv short              # 安装全部 5 个垫片到托管目录并自动加入 PATH
sysenv short --dir ~/bin  # 指定安装目录
sysenv short -f           # 覆盖已存在的垫片
sysenv short --temporary  # 不持久化 PATH，只打印可粘贴的片段
```



***

## 限制



* `http`：`--auth-type digest`、`--session`、`--stream`、`--ssl`、`--cert`、自定义 `--boundary` 未实现（见 `sysenv help http`）

* `path export` 的 `.reg` 仅适用于 Windows（Registry Editor 格式），Linux 交换备份请用 `.txt` / `.json`

* `link` 符号链接在 Windows 上需要开发者模式或管理员权限，无权限时自动降级为拷贝

* `ai` 数据来自 models.dev 公开接口，离线时使用 24h 缓存，缓存过期且无网络时报告错误

## 开发与验证



* `cargo test`：Windows 26 个单测、Linux 23 个单测（平台相关用例按 `cfg` 门控）

* 已分别在 Windows（msvc）与 Ubuntu（WSL 实机）完成 `cargo check`、`cargo test`、`cargo build --release` 与端到端冒烟

* 端到端验证覆盖：PATH 增删改查与注册表恢复、链接回退链、环境变量持久化 / 临时模式、http 全参数冒烟、ai 真实联网抓取与 canonical 优选、short 五种垫片实际执行

* `dev/` 目录提供本地冒烟工具：`test_server.ps1`（HttpListener 测试服务器，端口 18899）、`smoke_http.ps1` / `smoke_http2.ps1`、`test_body.json`