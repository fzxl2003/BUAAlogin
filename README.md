# 北航校园网原生自动登录

共享 Rust 认证内核，Windows Win32 与 macOS AppKit 原生界面，Linux Docker 运行编译后的命令行程序。发行版不包含 Python，运行时不启动 curl、PowerShell、ifconfig 或 ip 子进程。

## 各平台使用

- **Windows 10/11 x64**：完整解压 `BUAALogin-Windows-x64.zip`，双击 `BUAALogin.exe`；无需管理员权限或额外运行时。
- **macOS 12+ Apple Silicon**：解压 `BUAALogin-Mac-arm64.zip`，打开 `BUAALogin.app`。临时签名版本未公证，首次运行按系统提示允许打开。
- 桌面版使用平台原生控件，分为“连接”和“账号设置”两个 tab。连接页用一个动态按钮开始／停止，校园网认证、外网检测和网页兜底按行展示开关、测试结果与下次运行；最近活动固定显示 5 条，并可直接打开按日保存的日志文件夹。账号页管理凭据与开机自启，运行时锁定配置。关闭窗口隐藏至托盘／菜单栏，从菜单退出才停止运行。
- Windows 界面采用分段导航、白色信息卡片、清晰的中文字体和统一的蓝色主操作；保留原生输入、键盘导航和高对比度配色，不引入网页运行时、动画或刷新定时器。
- 凭据可选择保存至原生版独立的系统钥匙串／当前 Windows 用户 DPAPI 文件。首次使用必须重新输入；原生版不会读取或删除旧版凭据。
- 正常时默认每 300 秒检查认证记录和外网；外网异常时每 10 秒复核，连续三轮失败后分级恢复。网络变化合并 2 秒后检测，普通请求失败按 30、60、120、240、300 秒重试。无接口时等待系统网络通知，桌面睡眠期间暂停请求。
- 多网卡环境下认证固定走所选接口，独立绕过系统代理，保持 HTTPS 证书及主机名校验。指定接口不可用时等待该接口，不切换其他上联。

### 开机自启

在“账号设置”中勾选“开机自启”，默认关闭，作用于当前用户登录系统时。填写账号、密码和连接设置，勾选“记住账号密码”，再到“连接”页点击“开始自动连接”保存；从完整表单开启自启时也会保存配置。下次登录会隐藏到菜单栏／托盘并自动启动重连。凭据或配置缺失时打开账号设置页，不发起接口检测或认证请求；无可用接口时等待网络通知，失败保留递增退避。

macOS 13+ 使用 [SMAppService 系统登录项](https://developer.apple.com/documentation/servicemanagement/smappservice/mainapp)，需要系统允许时可点击“打开系统登录项设置”；macOS 12 使用当前用户 LaunchAgent。Windows 使用 [当前用户 Run 登录启动项](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys)，无需管理员权限。关闭自启只取消之后的登录启动，不停止当前重连。请先将应用放在固定位置再开启：macOS 建议“应用程序”文件夹，Windows 移动 EXE 后重新开关一次自启更新路径。

### Docker（原生预览）

将 `owner/repo` 替换为仓库名：

```sh
docker run -d --name buaalogin --restart unless-stopped --network host \
  -e USERNAME='你的账号' -e PASSWORD='你的密码' \
  ghcr.io/owner/repo:native-preview
```

镜像支持 Linux amd64/arm64。使用原生 Linux host 网络；macOS/Windows Docker Desktop 的虚拟网络不等价于宿主机校园网接口。

必须通过环境变量传入非空 `USERNAME` 和 `PASSWORD`，即使使用 `--status` 也不能省略；不提供交互输入。默认尝试全部符合条件的 IPv4 接口，为每个接口独立维护认证会话、检查周期和退避。指定接口可传 `-e INTERFACE=eth0` 或 `--interface eth0`。

保留命令行选项：`--status`、`--once`、`--interface`、`--gateway-ip`、`--interval`（10–86400 秒整数）、`--debug`、`--log-file`。`--debug` 输出脱敏状态事件，不输出原始请求、凭据或 token。容器默认以非 root 用户运行，日志文件需使用可写挂载路径。停止容器通过 SIGTERM 取消监控；可用 `docker logs -f buaalogin` 查看状态变化。

## 编译与发行

Rust 工具链由 `rust-toolchain.toml` 固定，依赖由 `Cargo.lock` 固定。

```sh
# 在 Apple Silicon Mac 上，需系统 Command Line Tools
./tools/build-macos.sh

# 在 Windows x64 上，需 Visual Studio C++ Build Tools
powershell -NoProfile -File tools/build-windows.ps1

# 在 Linux 上，需 libcurl 开发包与 pkg-config
cargo build --locked --release -p buaalogin

# 原生 Docker 镜像
docker build -t buaalogin:local .
```

桌面产物位于 `releases/`，macOS 解包构建目录为 `build/macos-native/BUAALogin.app`。Windows 静态链接 libcurl 与 CRT，使用 Schannel；macOS 链接系统 libcurl；Docker 链接发行版 libcurl/TLS 与 CA 证书。

push 自动构建两个桌面包和双架构镜像，不执行测试、格式或 lint 门禁；构建成功后发布 GitHub **预览 Release** 和 GHCR `native-preview`、`sha-<提交SHA>`，不自动覆盖已有 `latest`。完成 [实机与能耗验收](docs/acceptance.md) 后，可手动运行工作流并勾选 `promote_native`，发布正式 Release 和 `latest`。

[架构与 C ABI](docs/native-architecture.md) 说明内核接口、线程与资源生命周期。仓库仅保留原生实现；Python、PowerShell 桌面入口及旧打包产物已移除。

## Wi-Fi 自动连接（macOS / Windows）

每张 Wi-Fi 网卡可独立选择“发现 BUAA-WiFi 自动连接”，默认关闭。网卡即使未连接、未获取 IP 也会列出。macOS 从“Wi-Fi 自动连接…”窗口设置；Windows 选择“Wi-Fi 网卡”并勾选其选项。设置独立持久保存，与是否记住校园网密码无关。

选项在自动重连运行期间生效：发现精确 SSID `BUAA-WiFi` 时可从当前无线网络切换，已连接时不重复连接。启动、开启选项、断开及唤醒时发起一次定向扫描，后续使用系统扫描结果通知，不按认证检查周期反复扫描。单网卡无线连接失败后至少 30 秒才允许再次尝试；停止监控或睡眠时暂停自动连接。Linux/Docker 不提供此功能。

macOS 可能需要允许定位权限才能读取 Wi-Fi 信息；Windows 新版本也可能要求定位权限或 WLAN AutoConfig 服务。macOS 自动连接开放的 BUAA-WiFi；Windows 优先复用系统已有无线配置，没有配置时只直接连接开放网络。无线加密密码与校园网网页认证密码不同，程序不会将校园网密码用于 Wi-Fi。

## 外网检测与分级恢复

“网关已认证”和“外网可用”分别记录、展示。外网检测通过同一接口、绕过代理，向百度与腾讯首页发送 HTTPS HEAD 请求，保留证书校验且不跟随跳转；任一站点返回 2xx 即视为可用。每站连接超时 3 秒、总超时 5 秒。外网正常时不会因为网关状态读取失败而注销连接。

桌面端可分别关闭校园网认证和外网检测，两项共用检查间隔并分别显示最近／下次运行。关闭外网检测会同时关闭自动网页兜底；三个任务均可手动执行真实测试，其中网页测试会实际运行后台浏览器并验证外网。

连续三轮外网检测均失败后，先执行一次协议注销，等待 2 秒，重新读取门户 IP、ac_id 和 challenge 并强制登录；再最多验证三次外网，间隔 2 秒。恢复失败时，桌面版自动尝试一次后台网页注销和登录。最终以外网检测通过判定恢复，网页或网关的成功提示不能代替连通性验证。

同一网关、校园网 IP 的接口共享恢复记录。恢复链开始后有 10 分钟冷却，网络通知和接口会话重建不会清除冷却；期间继续检查外网，但不重复注销或启动浏览器。明确的密码错误、账号限制、欠费或验证码要求会停止自动升级，修改配置并重新开始后再试。

macOS、Windows 连接页为认证、外网检测和网页兜底分别提供“测试”；运行期间的测试进入同一串行队列。高级连接设置可填写浏览器**可执行文件**路径。默认查找已安装的 Chrome、Edge、Chromium（Windows 优先 Edge），不捆绑或安装浏览器；找不到时明确提示，并继续协议检测。

网页兜底仅在需要时启动独立的隐藏浏览器进程，使用临时用户目录，通过绑定当前网卡的本地 HTTPS CONNECT 代理访问 `gw.buaa.edu.cn:443`。正常联网时不运行浏览器，不操作用户现有窗口。单次网页操作和后续验证共用 90 秒预算；停止、睡眠、网络变化或退出会取消操作并清理进程和临时目录。识别到页面控件歧义、页面改版或验证码时明确失败，不猜测点击。

Linux/Docker 使用相同的外网检测和协议恢复，不包含网页兜底依赖。`--status` 只读取状态，JSON 事件分别输出 `authentication`、`connectivity` 和 `probes`；`--once` 最多执行一轮协议恢复，不启动浏览器。

当前网页驱动已提供通用 Srun 控件适配和本地真实浏览器测试；真实北航门户的 DOM、必要资源域名及多网卡/VPN 行为仍须校园网实机验收，发行保持预览版。

## 电脑接口 IP 与校园网 IP

本地 IP 从系统接口 API 获取，并用于绑定 HTTP 请求的源地址和出口网卡；认证 IP 从通过该接口访问的校园网门户页面获取，再与 challenge 的 client_ip 和状态接口的 online_ip 相互校验。

例如电脑为 `192.168.1.20`、路由器校园网侧为 `10.138.1.2`：电脑发出请求时源 IP 是 `192.168.1.20`，经过路由器 NAT 后网关看到 `10.138.1.2`，Srun 登录参数使用后者。两者允许不同，不再要求校园网 IP 等于电脑接口 IP。在线状态同时展示两种 IP；网关地址仍需能经所选网卡访问。共享同一 NAT 出口的设备通常共享该出口的认证状态，具体行为由网关决定。

可选 systemd 服务模板位于 `systemd/buaalogin.service`，使用 `/usr/local/bin/buaalogin` 与 `/etc/buaalogin.env` 中的凭据，日志写入 journal。
