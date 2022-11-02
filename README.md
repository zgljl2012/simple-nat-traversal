# CPChain 内网穿透工具

用于外网访问在 CPChain 内网部署的节点服务和区块链浏览器服务。

## Setup

```bash

source ./scripts/clear-proxy.sh
source ./scripts/set-proxy.sh

curl -i localhost:5001

curl -i -X POST localhost:5001 -d'{"Hello": "World!"}'

```
