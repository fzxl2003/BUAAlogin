# 北航校园网原生自动登录

共享 Rust 认证内核，Windows Win32 与 macOS AppKit 原生界面，Linux Docker 提供内嵌网页管理端。发行版不包含 Python 或 Node.js，运行时不启动 curl、PowerShell、ifconfig 或 ip 子进程。

## 各平台使用

- **Windows 10/11 x64**：完整解压 `BUAALogin-Windows-x64.zip`，双击 `BUAALogin.exe`；无需管理员权限或额外运行时。
- **macOS 12+ Apple Silicon**：解压 `BUAALogin-Mac-arm64.zip`，打开 `BUAALogin.app`。临时签名版本未公证，首次运行按系统提示允许打开。
- macOS 与 Windows 原生版均分为“连接”和“设置”两个 tab。连接页用一个动态按钮开始／停止，并按三行展示账号、接口、周期、地址与门户累计流量；账号通过弹窗保存，接口设置支持多选及逐接口网关。最近活动、总流量、开机自启和后台浏览器路径集中在“设置”页。关闭窗口隐藏至菜单栏／托盘，从菜单退出才停止运行。
- Windows 界面采用分段导航、白色信息卡片、清晰的中文字体和统一的蓝色主操作；保留原生输入、键盘导航和高对比度配色，不引入网页运行时、动画或 UI 刷新定时器。
- 凭据可选择使用应用内轻量加密保存，不调用 macOS 钥匙串或 Windows DPAPI，因此更新应用不会触发系统密码授权。此格式主要用于避免明文存储，不等同于系统级凭据保护；首次切换到该版本需要重新输入一次。
- 默认每 300 秒检查认证记录和外网；正常检查、失败重试、异常复核和恢复后的下一轮共用同一周期，连续三轮外网失败后分级恢复。Mac 连接页“检查周期”旁的“设置”可修改周期（10–86400 秒），保存后立即持久化，运行中在当前操作结束后更新下一次执行时间。网络变化合并 2 秒后检测。无接口时等待系统网络通知，桌面睡眠期间暂停请求。
- 多网卡环境下认证固定走所选接口，独立绕过系统代理，保持 HTTPS 证书及主机名校验。指定接口不可用时等待该接口，不切换其他上联。

### 开机自启

在“设置”中勾选“开机自启”，默认关闭，作用于当前用户登录系统时。先从连接页的账号弹窗填写账号、密码并勾选“记住账号密码”。下次登录会隐藏到菜单栏／托盘并自动启动重连；凭据缺失时打开账号弹窗，不发起认证请求。

macOS 13+ 使用 [SMAppService 系统登录项](https://developer.apple.com/documentation/servicemanagement/smappservice/mainapp)，需要系统允许时可点击“打开系统登录项设置”；macOS 12 使用当前用户 LaunchAgent。Windows 使用 [当前用户 Run 登录启动项](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys)，无需管理员权限。关闭自启只取消之后的登录启动，不停止当前重连。请先将应用放在固定位置再开启：macOS 建议“应用程序”文件夹，Windows 移动 EXE 后重新开关一次自启更新路径。

### Docker 网页版

将 `owner/repo` 替换为仓库名：

```sh
docker run -d --name buaalogin --restart unless-stopped --network host \
  -v buaalogin-data:/data \
  ghcr.io/owner/repo:native-preview

docker logs buaalogin
```

镜像支持 Linux amd64/arm64，默认监听 `0.0.0.0:18799`。首次启动后，从 `docker logs buaalogin` 复制一次性设置码，打开 `http://服务器地址:18799` 创建网页管理密码。镜像以 UID/GID 65532 非 root 用户运行，配置、凭据和最近 30 天日志存放在 `/data`；务必挂载数据卷，否则重建容器后会重新进入首次设置。

仓库的 [`compose.yaml`](compose.yaml) 是 host-network 示例。使用原生 Linux host 网络时，服务才能直接看到宿主机校园网接口；macOS/Windows Docker Desktop 的虚拟网络不等价于宿主机接口。接口留空表示自动选择，显式多选后每个接口独立维护认证会话并使用自己的可选网关。容器会标注 Wi‑Fi/有线类型，但不能扫描、切换或连接宿主机 Wi‑Fi。

网页提供“连接”和“设置”两页：账号、检查周期、流量进度、接口多选与逐接口网关、校园网认证/外网检测及测试、活动记录、日志下载和管理密码修改。运行中会锁定账号、接口和任务开关，但仍可修改检查周期与总流量。服务使用 SSE 推送状态，不需要定时刷新页面；镜像不包含旧 CLI、Chromium、开机自启、后台浏览器或网页兜底。Debian Bookworm 的 [Chromium 软件包](https://packages.debian.org/bookworm/amd64/chromium) 本体已超过约定的 50MB 增量，因此不加入镜像。

服务只提供 HTTP，并默认允许局域网访问。**请仅在可信局域网中使用。** 公网部署必须自行配置 HTTPS 反向代理、来源限制和防火墙；设置码、管理密码和校园网密码都不应经不可信网络明文传输。修改监听地址/端口可设置 `WEB_BIND=0.0.0.0:18080`，并相应开放该端口；内置健康检查会自动使用同一地址。

从旧 Docker 配置迁移时，可在一个全新的 `/data` 卷首次启动时继续传入：

```sh
docker run -d --name buaalogin --restart unless-stopped --network host \
  -v buaalogin-data:/data \
  -e USERNAME='你的账号' -e PASSWORD='你的密码' -e INTERFACE='eth0' \
  ghcr.io/owner/repo:native-preview
```

完整账号会被加密导入并自动开始监控。导入只在 `/data/settings.json` 不存在时执行；之后网页保存的配置始终优先，环境变量不会覆盖它。显式点击“开始”会持久化期望运行状态，正常重启和 SIGTERM 后自动恢复；显式停止则清除。可用 `docker logs -f buaalogin` 查看设置码和进程级状态，详细脱敏日志从设置页下载。

## 编译与发行

Rust 工具链由 `rust-toolchain.toml` 固定，依赖由 `Cargo.lock` 固定。

```sh
# 在 Apple Silicon Mac 上，需系统 Command Line Tools
./tools/build-macos.sh

# 在 Windows x64 上，需 Visual Studio C++ Build Tools
powershell -NoProfile -File tools/build-windows.ps1

# Linux 网页服务，需 libcurl 开发包与 pkg-config
cargo build --locked --release -p buaalogin-web

# 网页版 Docker 镜像
docker build -t buaalogin:local .
```

桌面产物位于 `releases/`，macOS 解包构建目录为 `build/macos-native/BUAALogin.app`。Windows 静态链接 libcurl 与 CRT，使用 Schannel；macOS 链接系统 libcurl；Docker 的单一网页服务二进制链接发行版 libcurl/TLS 与 CA 证书。

push 自动构建两个桌面包和双架构镜像，不执行测试、格式或 lint 门禁；构建成功后发布 GitHub **预览 Release** 和 GHCR `native-preview`、`sha-<提交SHA>`，不自动覆盖已有 `latest`。完成 [实机与能耗验收](docs/acceptance.md) 后，可手动运行工作流并勾选 `promote_native`，发布正式 Release 和 `latest`。

[架构与 C ABI](docs/native-architecture.md) 说明内核接口、线程与资源生命周期。仓库仅保留原生实现；Python、PowerShell 桌面入口及旧打包产物已移除。

## Wi-Fi 自动连接（macOS / Windows）

macOS 与 Windows 的接口列表均明确标注 Wi-Fi 与有线类型；Wi-Fi 网卡即使尚未获取 IP 或未检测到校园网也会保留，以便在接口详情中配置自动连接。每张 Wi-Fi 网卡默认提供 `BUAA-WiFi` 和 `BUAA-WiFi-Free`，也可填写一个自定义 SSID；有线接口不显示 Wi-Fi 选项。设置独立持久保存，与是否记住校园网密码无关。

选项在自动重连运行期间生效：发现任一配置的精确 SSID 时可从当前无线网络切换，已连接到目标网络时不重复连接。启动、开启选项、断开及唤醒时发起一次扫描，后续使用系统扫描结果通知，不按认证检查周期反复扫描。单网卡无线连接失败后至少 30 秒才允许再次尝试；停止监控或睡眠时暂停自动连接。Linux/Docker 不提供此功能。

macOS 可能需要允许定位权限才能读取 Wi-Fi 信息；Windows 新版本也可能要求定位权限或 WLAN AutoConfig 服务。macOS 自动连接开放的 BUAA-WiFi；Windows 优先复用系统已有无线配置，没有配置时只直接连接开放网络。无线加密密码与校园网网页认证密码不同，程序不会将校园网密码用于 Wi-Fi。

## 外网检测与分级恢复

“网关已认证”和“外网可用”分别记录、展示。外网检测通过同一接口、绕过代理，向百度与腾讯首页发送 HTTPS HEAD 请求，保留证书校验且不跟随跳转；任一站点返回 2xx 即视为可用。每站连接超时 3 秒、总超时 5 秒。外网正常时不会因为网关状态读取失败而注销连接。

桌面端可分别关闭校园网认证和外网检测，两项共用检查间隔并分别显示最近／下次运行。关闭外网检测会同时关闭自动网页兜底；三个任务均可手动执行真实测试，其中网页测试会实际运行后台浏览器并验证外网。

连续三轮外网检测均失败后，先执行一次协议注销，等待 2 秒，重新读取门户 IP、ac_id 和 challenge 并强制登录；随后验证一次外网。协议恢复失败时，桌面版自动尝试一次后台网页注销和登录并验证外网；仍失败则在下一检查周期重试。最终以外网检测通过判定恢复，网页或网关的成功提示不能代替连通性验证。

同一网关、校园网 IP 的接口共享恢复记录。恢复完成后按同一检查周期安排下一轮，同一出口在一个检查周期内不会重复恢复，网络通知和接口会话重建不会清除记录。明确的密码错误、账号限制、欠费或验证码要求会停止自动升级，修改配置并重新开始后再试。

macOS、Windows 连接页为认证、外网检测和网页兜底分别提供“测试”；运行期间的测试进入同一串行队列。两端均在“设置”页填写浏览器**可执行文件**路径。默认查找已安装的 Chrome、Edge、Chromium（Windows 优先 Edge），不捆绑或安装浏览器；找不到时明确提示，并继续协议检测。

网页兜底仅在需要时启动独立的隐藏浏览器进程，使用临时用户目录，通过绑定当前网卡的本地 HTTPS CONNECT 代理访问 `gw.buaa.edu.cn:443`。正常联网时不运行浏览器，不操作用户现有窗口。单次网页操作和后续验证共用 90 秒预算；停止、睡眠、网络变化或退出会取消操作并清理进程和临时目录。识别到页面控件歧义、页面改版或验证码时明确失败，不猜测点击。

网页兜底先判断入口：若显示“注销”，优先注销当前会话并确认已返回登录页，才填写并重新登录；若已是登录页，则直接登录。注销尚未完成时不会填写或提交，即使页面仍保留登录输入框。页面脚本及按钮须初始化完成，注销与提交登录后各最多等待 30 秒确认状态切换，再刷新网关认证记录并验证外网。提交动作本身不代表认证成功。登录错误弹窗会保留安全的错误码，未知提示不自动确认。恢复日志分别记录页面判断、注销、登录、网关认证状态与每轮外网探测结果。

Linux/Docker 使用相同的外网检测和协议恢复，不包含网页兜底依赖。`--status` 只读取状态，JSON 事件分别输出 `authentication`、`connectivity` 和 `probes`；`--once` 最多执行一轮协议恢复，不启动浏览器。

当前网页驱动已提供通用 Srun 控件适配和本地真实浏览器测试；真实北航门户的 DOM、必要资源域名及多网卡/VPN 行为仍须校园网实机验收，发行保持预览版。

## 电脑接口 IP 与校园网 IP

本地 IP 从系统接口 API 获取，并用于绑定 HTTP 请求的源地址和出口网卡；认证 IP 从通过该接口访问的校园网门户页面获取，再与 challenge 的 client_ip 和状态接口的 online_ip 相互校验。

例如电脑为 `192.168.1.20`、路由器校园网侧为 `10.138.1.2`：电脑发出请求时源 IP 是 `192.168.1.20`，经过路由器 NAT 后网关看到 `10.138.1.2`，Srun 登录参数使用后者。两者允许不同，不再要求校园网 IP 等于电脑接口 IP。在线状态同时展示两种 IP；网关地址仍需能经所选网卡访问。共享同一 NAT 出口的设备通常共享该出口的认证状态，具体行为由网关决定。

可选 systemd 服务模板位于 `systemd/buaalogin.service`，使用 `/usr/local/bin/buaalogin` 与 `/etc/buaalogin.env` 中的凭据，日志写入 journal。
