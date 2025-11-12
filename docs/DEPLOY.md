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
- 生产环境务必更换 `JWT_SECRET`、`PREAMBLE_SECRET_KEY`、`DEFAULT_ADMIN_PASSWORD`。
- 如使用兼容网关，需同步修改 `OPENAI_BASE_URL` 与 `EMBEDDING_BASE_URL`。


### 4. 安装数据库

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

### 4. 启动后端
```bash
./rig-rag 
```

服务默认监听：`http://0.0.0.0:3000`

### 5. 管理后台地址
- 管理后台：`http://<你的域名或IP>:3000/admin`
- 首次登录使用 `.env` 的 `DEFAULT_ADMIN_PASSWORD`，登录后请立即修改。

### 6. 构建与提供前端静态资源（可选）
仓库已提供 `static/` 目录的打包产物。若需要从 `frontend/js` 重新构建压缩版 JS：
```bash
# 全局安装 terser（或在项目内安装）
npm i -g terser  # 或 npm i terser

# 运行构建脚本（输出到 static/js）
npm run build
```

部署时只需让后端或你的 Web 服务器能访问 `static/` 目录下的文件。

### 7. 在任意网站集成 Chat 组件（chat JS）
在目标站点页面中加入：
```html
<script src="/static/js/chatbot.js"></script>
<script>
  const chatbot = new RigChat({
    apiBase: "",            // 留空=当前域名；或填入你的后端 API 域名
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


### 8. 常见问题
- 无法访问管理后台：确认服务已启动、端口开放、防火墙配置正确。
- 模型报错或无响应：检查 `OPENAI_API_KEY` 与 `OPENAI_BASE_URL` 是否正确。
- 向量检索异常：确认 `data` 目录有读写权限，且磁盘空间充足。
- 跨域问题：若前后端不同域名，请在后端开启相应的 CORS（若有需要）。


### 9. 查看 Qdrant 管理界面
http://localhost:6333/dashboard#/collections/rig_documents



