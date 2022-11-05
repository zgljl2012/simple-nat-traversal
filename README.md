# 内网穿透工具

用于外网访问在内网部署的服务与机器。

## Setup

```bash

source ./scripts/clear-proxy.sh
source ./scripts/set-proxy.sh

curl -i localhost:5001

curl -i -X POST localhost:5001 -d'{"Hello": "World!"}'

```

## FAQ

### 修改日志级别

```bash

# info,debug,error,warn
export RUST_LOG=info

./simple-nat-traversal <....>

```

## Reference

- [SSH Protocol – Secure Remote Login and File Transfer](https://www.ssh.com/academy/ssh/protocol)
