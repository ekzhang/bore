# bore

一个用 Rust 编写的现代化 TCP 隧道工具，用于将本地端口暴露到远程服务器，绕过标准的 NAT 连接防火墙。

## 简介

本版本基于 [bore](https://github.com/ekzhang/bore) 项目修改，主要增加了**自定义控制端口**功能。

原版 bore 默认使用固定的控制端口 7835，在某些网络环境下可能因端口被占用或被防火墙阻止而无法使用。本版本允许用户通过命令行参数自定义控制端口，解决了这一问题。

## 主要特性

- **自定义控制端口**：支持通过 `--control-port` 参数指定控制端口（默认 7835）
- **环境变量支持**：可通过 `BORE_CONTROL_PORT` 环境变量设置默认控制端口
- **向后兼容**：默认行为与原版完全一致，不会影响现有部署
- **简洁高效**：基于原版约 400 行安全的异步 Rust 代码

## 安装

### 从源码编译

```bash
# 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 编译项目
cargo build --release

# 运行
./target/release/bore
```

### 在 Armbian 上编译

```bash
# 安装 Rust 和依赖
sudo apt update
sudo apt install -y build-essential git curl pkg-config
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# 编译
cargo build --release
```

## 使用方法

### 服务器端

```bash
# 使用默认控制端口 7835
bore server

# 使用自定义控制端口 19163
bore server --control-port 19163

# 指定绑定地址和控制端口
bore server --bind-addr 0.0.0.0 --control-port 19163

# 添加认证密钥（可选）
bore server --control-port 19163 --secret my_secret
```

服务器端完整选项：
```
Usage: bore server [OPTIONS]

Options:
      --min-port <MIN_PORT>          最小可接受的 TCP 端口号 [env: BORE_MIN_PORT=] [默认: 1024]
      --max-port <MAX_PORT>          最大可接受的 TCP 端口号 [env: BORE_MAX_PORT=] [默认: 65535]
  -s, --secret <SECRET>              认证密钥 [env: BORE_SECRET]
      --bind-addr <BIND_ADDR>        绑定的 IP 地址 [默认: 0.0.0.0]
      --bind-tunnels <BIND_TUNNELS>  隧道监听的 IP 地址（默认为 --bind-addr）
      --control-port <PORT>          控制端口 [env: BORE_CONTROL_PORT] [默认: 7835]
  -h, --help                         打印帮助信息
```

### 客户端

```bash
# 使用默认控制端口 7835
bore local 8000 --to server.example.com

# 使用自定义控制端口 19163
bore local 5173 --to wyschina.vip --port 8089 --control-port 19163

# 添加认证密钥（可选）
bore local 5173 --to wyschina.vip --port 8089 --secret ardondon --control-port 19163
```

客户端完整选项：
```
Usage: bore local [OPTIONS] --to <TO> <LOCAL_PORT>

参数:
  <LOCAL_PORT>  要暴露的本地端口 [env: BORE_LOCAL_PORT=]

选项:
  -l, --local-host <HOST>  要暴露的本地主机 [默认: localhost]
  -t, --to <TO>            远程服务器地址 [env: BORE_SERVER=]
  -p, --port <PORT>        远程服务器上选择的端口 [默认: 0]
  -s, --secret <SECRET>    认证密钥 [env: BORE_SECRET]
      --control-port <PORT> 控制端口 [env: BORE_CONTROL_PORT] [默认: 7835]
  -h, --help               打印帮助信息
```

## 配置为 systemd 服务

在服务器上创建服务文件：

```bash
sudo nano /etc/systemd/system/bore-server.service
```

添加以下内容：
```ini
[Unit]
Description=Bore TCP Tunnel Server
After=network.target

[Service]
Type=simple
User=ubuntu
ExecStart=/home/ubuntu/bore/bore server --control-port 19163
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

启动服务：
```bash
sudo systemctl daemon-reload
sudo systemctl enable bore-server
sudo systemctl start bore-server
sudo systemctl status bore-server
```

## 协议说明

控制端口用于按需创建新连接。客户端初始化时，通过 TCP 控制端口向服务器发送"Hello"消息，请求代理指定的远程端口。服务器响应确认并开始监听外部 TCP 连接。

当服务器获得远程端口的连接时，生成一个安全的 UUID 发送给客户端。客户端再打开单独的 TCP 流，发送包含 UUID 的"Accept"消息。服务器然后将两个连接相互代理。

为避免内存泄漏，如果客户端未在 10 秒内接受连接，服务器将丢弃传入连接。

## 认证

在自定义部署中，可以通过设置密钥来防止服务器被他人使用。协议要求客户端在每次 TCP 连接上通过 HMAC 代码回答随机挑战来验证密钥。

```bash
# 服务器端
bore server --secret my_secret_string

# 客户端
bore local <LOCAL_PORT> --to <TO> --secret my_secret_string
```

如果命令行中未指定密钥，bore 还会尝试从 `BORE_SECRET` 环境变量中读取。

## 常见问题

### 连接超时

- 检查服务器防火墙是否允许控制端口的访问
- 验证服务器地址和控制端口是否正确
- 确认服务器上的 bore 服务是否正常运行

### Vite 开发服务器访问被阻止

在 `vite.config.js` 中添加 `allowedHosts` 配置：
```javascript
export default {
  server: {
    allowedHosts: ['your-domain.com']
  }
}
```

### 客户端和服务器控制端口必须一致

```bash
# 服务器
bore server --control-port 19163

# 客户端
bore local 5173 --to server --control-port 19163
```

## 支持的协议

作为 TCP 隧道工具，bore 支持所有基于 TCP 的协议：
- HTTP/HTTPS
- WebSocket
- SSH
- 数据库连接
- 其他 TCP 协议

## 关于

本项目基于 [bore](https://github.com/ekzhang/bore) 修改，感谢原作者的出色工作。

许可证：[MIT](LICENSE)
