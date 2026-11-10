## 部署指南

本文档说明如何在服务器上部署后端、配置环境变量、进入管理后台，以及如何在任意网站集成聊天组件（chat JS）。

### 1. 获取 AI 与嵌入模型的 API Key
支持 OpenAI 及兼容平台（如自托管或第三方兼容网关）。
- OpenAI Key：在 OpenAI 账户后台生成
- 嵌入模型 Key：可与上方相同或来自兼容供应商

记录以下信息：
- OPENAI_API_KEY
- OPENAI_BASE_URL（默认 `https://api.openai.com/v1`，兼容网关请改为其地址）
- OPENAI_MODEL（如 `gpt-4o` 或 `gpt-3.5-turbo`）
- EMBEDDING_API_KEY（可与 OPENAI_API_KEY 相同）
- EMBEDDING_BASE_URL（默认 `https://api.openai.com/v1`）
- EMBEDDING_MODEL（如 `text-embedding-3-large` 或 `text-embedding-ada-002`）

### 2. 配置环境变量
拷贝示例文件：
```bash
cp env.example .env
```

根据需要编辑 `.env`


注意事项：
- 生产环境必须设置随机 `JWT_SECRET`（建议 `openssl rand -hex 32`；占位值会直接拒绝启动），并更换 `PREAMBLE_SECRET_KEY`、`DEFAULT_ADMIN_PASSWORD`。
- 如使用兼容网关，需同步修改 `OPENAI_BASE_URL` 与 `EMBEDDING_BASE_URL`。


### 3. 安装数据库

- 使用docker
```bash
docker run -p 6333:6333 -p 6334:6334 \
    -v "$(pwd)/qdrant_storage:/qdrant/storage:z" \
    qdrant/qdrant
```

- 使用安装包安装
```bash
https://github.com/qdrant/qdrant/releases

https://qdrant.tech/documentation/guides/configuration/
```

本api是用安装包qdrant  rag 

### 4. 本地 Markdown 知识库目录（可选）
在 `.env` 中配置：
```bash
DOCUMENTS_DIR=data/documents
# 可选：排除某些子目录（逗号分隔）
DOCUMENTS_EXCLUDE_DIRS=drafts,.git,internal/wip
```
将该目录下的 `.md` 文件（支持子目录）放入后，**启动时会自动同步进向量库并参与检索**。运行中修改/增删文件也会被轮询检测到（默认每 30 秒，可用 `DOCUMENTS_SYNC_INTERVAL_SECS` 调整），只重嵌有变更的文件并自动重建 Agent。`DOCUMENTS_EXCLUDE_DIRS` 中：纯目录名匹配任意层级；带 `/` 的按相对路径前缀排除。

### 5. 启动后端
本地临时启动：
```bash
./rig-rag
```

服务默认监听：`http://0.0.0.0:3000`

### 5.1 CentOS / RHEL 开机自启（systemd）
部署目录示例 `/opt/rig-rag`：

```text
/opt/rig-rag/
  ├── rig-rag          # 二进制
  ├── .env
  ├── data/
  ├── static/
  └── dep/             # 通用运维脚本（改 config.sh 即可复用到其他程序）
      ├── config.sh            # 唯一需改：APP_NAME / APP_BIN / APP_DESC
      ├── app.service          # systemd 模板
      ├── install-service.sh
      ├── uninstall-service.sh
      ├── start.sh
      ├── stop.sh
      └── logs.sh
```

```bash
# 准备配置
cp docs/env.example .env
vi .env

# 安装并启用开机自启（需 root）
chmod +x dep/*.sh
sudo ./dep/install-service.sh
```

常用命令：
```bash
./dep/start.sh                 # 启动
./dep/start.sh restart         # 重启
./dep/stop.sh                  # 停止
./dep/logs.sh                  # 实时看 journal 日志
./dep/logs.sh -n 200           # 最近 200 行后跟踪
./dep/logs.sh --no-follow      # 只看最近日志不跟踪
sudo systemctl status rig-rag
sudo systemctl restart rig-rag
sudo ./dep/uninstall-service.sh   # 卸载自启（不删数据）
```

说明：
- 复用到其他程序：复制整个 `dep/`，只改 `dep/config.sh` 里的 `APP_NAME`（及可选 `APP_BIN` / `APP_DESC`）。
- `install-service.sh` 以部署根目录为 `WorkingDirectory`，生成 `/etc/systemd/system/<APP_NAME>.service` 并 `enable` + `restart`。
- 日志统一走 systemd journal（`journalctl -u <APP_NAME>`），不写 `app.log`。

### 6. 管理后台地址
- 管理后台：`http://<你的域名或IP>:3000/admin`
- 首次登录使用 `.env` 的 `DEFAULT_ADMIN_PASSWORD`，登录后请立即修改。

### 7. 构建与提供前端静态资源（可选）
仓库已提供 `static/` 目录的打包产物。若需要从 `frontend/js` 重新构建压缩版 JS：
```bash
# 全局安装 terser（或在项目内安装）
npm i -g terser  # 或 npm i terser

# 运行构建脚本（输出到 static/js）
npm run build
```

部署时只需让后端或你的 Web 服务器能访问 `static/` 目录下的文件。

### 8. 在任意网站集成 Chat 组件（chat JS）
在目标站点页面中加入：
```html
<!-- head -->
<script src="https://chat.xxx.com/static/js/chatbot.js"></script>
<!-- body -->
<script>
  const chatbot = new RigChat({
    apiBase: "https://chat.xxx.com",            // 留空=当前域名；或填入你的后端 API 域名
    title: "AI Assistant",
    welcomeMessage: "👋 您好，我是 AI Assistant！",
    buttonIcon: "ai",
    theme: "dark",          // 或 "light"
    position: "right",      // 或 "left"
    placeholder: "请输入你的问题..."
  });
</script>
```

说明：
- 如果你的页面与后端不在同一域名，设置 `apiBase` 为后端地址（如 `https://api.example.com`）。
- 部署 `chatbot.js` 时，确保可通过站点路径 `/static/js/chatbot.js` 访问，或调整 `<script src>` 为你实际的静态路径。


### 9. 常见问题
- 无法访问管理后台：确认服务已启动、端口开放、防火墙配置正确。
- 模型报错或无响应：检查 `OPENAI_API_KEY` 与 `OPENAI_BASE_URL` 是否正确。
- 向量检索异常：确认 `data` 目录有读写权限，且磁盘空间充足。
- 跨域问题：若前后端不同域名，请在后端开启相应的 CORS（若有需要）。
- 聊天感觉不是流式：若前面有 Nginx/CDN，需关闭代理缓冲，例如：
  ```nginx
  location /api/chat/stream {
      proxy_pass http://127.0.0.1:3000;
      proxy_http_version 1.1;
      proxy_set_header Connection "";
      proxy_buffering off;
      proxy_cache off;
      chunked_transfer_encoding on;
  }
  ```
  后端已返回 `X-Accel-Buffering: no`；同时确认 `.env` 中 `RAG_TOP_K` 不要过大（默认 5）。


### 10. 查看 Qdrant 管理界面
http://localhost:6333/dashboard#/collections/rig_documents



