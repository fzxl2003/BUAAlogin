## 各平台用法

- **Windows x64**：解压 Release 中的 Windows 包，双击 `Start.cmd`。
- **Mac ARM**：解压 Mac 包，打开 `BUAALogin.app`；未公证版本首次运行可在系统设置中允许打开。
- 桌面版输入账号密码，选择校园网接口或“自动选择”，点击“检测校园网”核对，再点击“开始自动重连”。支持加密记住凭据和最小化至托盘；从托盘菜单退出。
- 默认每 300 秒检查，掉线重连；失败后逐步延长重试至最多 300 秒，等待期间休眠。
- **Docker（Linux）**：将下方 `owner/repo` 替换为仓库名。必须在 `docker run` 时传入非空账号密码，否则立即报错退出；不提供交互输入。使用 host 网络访问宿主机的校园网接口。

```sh
docker run -d --name buaalogin --restart unless-stopped --network host \
  -e USERNAME='你的账号' -e PASSWORD='你的密码' ghcr.io/owner/repo:latest
```

默认依次尝试所有已连接的 IPv4 接口（不含回环），为可用的校园网接口登录并定期检查。指定接口可额外传入 `-e INTERFACE=eth0`；查看日志使用 `docker logs -f buaalogin`。

## 各平台编译方法

需要 Python 3.11+；Windows 打包时会下载官方嵌入式运行时，无需额外依赖。

```sh
# Windows 包（可在任意系统生成）
python tools/build.py windows

# Mac ARM：在 Apple Silicon Mac 上执行
python3 -m pip install -r requirements-build.txt
python3 tools/build.py macos

# Docker 镜像；或生成 Docker 源码发行包
docker build -t buaalogin:local .
python3 tools/build.py docker
```

产物位于 `releases/`。每次 push 自动构建：两个桌面包上传 GitHub Release，amd64/arm64 镜像推送至 `ghcr.io/<仓库名>:latest` 和 `:sha-<提交SHA>`。工作流使用自带的 `GITHUB_TOKEN`，须允许 Release/Packages 写入；GHCR 首次发布默认私有，需要公开下载时将包设为 Public。
