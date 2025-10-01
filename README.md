# Rig Demo - 文档管理增强版

这是一个基于 Rig 框架的 AI 聊天机器人演示，支持 SurrealDB 存储和网页文档管理功能。

## 🚀 功能特性

### 核心功能
- 🤖 基于 RAG (检索增强生成) 的 AI 聊天机器人
- 📚 支持多种文档格式 (Markdown, TXT, JSON, CSV)
- 💬 智能中英文语言检测和回复
- 📝 聊天历史记录管理

### 新增功能
- 🗄️ SurrealDB 数据库存储支持
- 🌐 网页文档管理界面
- 📤 文件上传功能
- ✏️ 在线文档编辑
- ⚙️ Preamble 配置管理
- 🔍 文档搜索和查看

## 📋 环境要求

- Rust 1.70+
- SurrealDB (可选)
- OpenAI API 密钥

## 🛠️ 快速开始

### 1. 克隆项目
```bash
git clone <your-repo-url>
cd rig-demo
```

### 2. 配置环境变量
复制环境变量示例文件：
```bash
cp env.example .env
```

编辑 `.env` 文件，填入你的配置：
```env
# OpenAI API配置
OPENAI_API_KEY=your_openai_api_key_here
OPENAI_BASE_URL=https://api.openai.com/v1
OPENAI_MODEL=gpt-3.5-turbo

# 嵌入模型配置
EMBEDDING_API_KEY=your_embedding_api_key_here
EMBEDDING_BASE_URL=https://api.openai.com/v1
EMBEDDING_MODEL=text-embedding-ada-002

# Agent配置
TEMPERATURE=0.1
PREAMBLE_FILE=documents/preamble.txt
DOCUMENTS_DIR=documents

LANCEDB_PATH=data/lancedb


```


### 3. 运行项目
```bash
cargo run
```

服务器将在 `http://127.0.0.1:3000` 启动。

## 🌐 使用界面

### 聊天界面
访问 `http://127.0.0.1:3000` 使用 AI 聊天功能。

### 文档管理界面
访问 `http://127.0.0.1:3000/admin` 进行文档和配置管理：

- **📄 文档管理**: 查看、编辑、删除已上传的文档
- **📤 上传文档**: 支持拖拽上传或手动创建文档
- **⚙️ Preamble配置**: 修改 AI 助手的系统提示词


## 🔌 API 端点

### 聊天 API
- `POST /api/chat` - 发送聊天消息
- `GET /api/history/{user_id}` - 获取聊天历史

### 文档管理 API (需要启用 SurrealDB)
- `GET /api/documents` - 获取文档列表
- `POST /api/documents` - 创建新文档
- `GET /api/documents/{id}` - 获取文档详情
- `PUT /api/documents/{id}` - 更新文档
- `DELETE /api/documents/{id}` - 删除文档
- `POST /api/documents/upload` - 上传文档

### Preamble 配置 API (需要启用 SurrealDB)
- `GET /api/preamble` - 获取当前 Preamble
- `PUT /api/preamble` - 更新 Preamble

## 🛡️ 安全注意事项

1. **API 密钥安全**: 不要在代码中硬编码 API 密钥
2. **CORS 配置**: 生产环境中请配置适当的 CORS 策略
3. **文件上传**: 当前实现仅支持文本文件，请根据需要添加文件类型验证
4. **数据库访问**: LanceDB 配置请使用强密码

## 🤝 贡献指南

1. Fork 项目
2. 创建功能分支 (`git checkout -b feature/AmazingFeature`)
3. 提交更改 (`git commit -m 'Add some AmazingFeature'`)
4. 推送到分支 (`git push origin feature/AmazingFeature`)
5. 开启 Pull Request

## 📄 许可证

本项目基于 MIT 许可证 - 查看 [LICENSE](LICENSE) 文件了解详情。

## 🙏 致谢

- [Rig Framework](https://github.com/0xPlaygrounds/rig) - 强大的 Rust AI 框架
- [LanceDB](https://lancedb.com/) - 现代多模型数据库
- [Axum](https://github.com/tokio-rs/axum) - 高性能异步 Web 框架