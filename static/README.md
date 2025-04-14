# Rig Chat UI

Rig Chat 是一个使用 Tailwind CSS 构建的前端聊天 UI 组件，提供了完整的聊天界面和可嵌入式组件。

## 文件结构

```
static/
├── index.html          # 完整的聊天界面
├── chatbot.js          # 可嵌入式聊天组件
└── chatbot-embed.html  # 嵌入示例
```

## 使用方法


### 1. 嵌入式组件

要将聊天组件嵌入到其他网站：

1. 构建 CSS 并部署 `chatbot.js` 到您的服务器。

2. 在目标网站中添加以下代码：

```html
<script src="/static/chatbot.js"></script>
<script>
   // 创建聊天组件实例
   const chatbot = new RigChat({
      apiBase: "", // 使用当前网站的API
      title: "Rig Assistant",
      welcomeMessage: "👋 您好，我是Rig Assistant，很高兴为您服务！",
      buttonIcon: "ai",
      theme: "dark",
      position: "right",
      placeholder: "请输入您的问题..."
   });
</script>
```

查看 `chatbot-embed.html` 了解更多嵌入选项和示例。


## 特性

- 深色/浅色模式支持
- 响应式设计，适配移动和桌面设备
- Markdown 文本渲染支持（包括代码高亮）
- 可嵌入到任何网站的独立组件
- 聊天历史记录保存
- 自定义主题和样式选项

## 嵌入组件配置选项

| 选项 | 默认值 | 说明 |
|------|-------|------|
| apiBase | 当前域名 | Rig API 的基础 URL |
| theme | "light" | 主题颜色 ("light" 或 "dark") |
| position | "right" | 聊天窗口位置 ("right" 或 "left") |
| welcomeMessage | "Welcome to Rig Assistant! How can I help you today?" | 初始欢迎消息 |
| title | "Rig Assistant" | 聊天窗口标题 |
| buttonIcon | "ai" | 聊天按钮图标名称 |
| placeholder | "Type your message..." | 输入框提示文字 |
| containerId | "rig-chat-container" | 聊天组件容器的 ID |

## 自定义样式

嵌入组件使用 CSS 变量定义样式，可以通过覆盖这些变量来自定义外观：

```css
:root {
    --bg-primary: #ffffff;    /* 主背景色 */
    --bg-secondary: #f3f4f6;  /* 次背景色 */
    --bg-accent: #0ea5e9;     /* 主题色 */
    --text-primary: #1f2937;  /* 主文本色 */
    --user-bubble: #0ea5e9;   /* 用户消息气泡 */
    --bot-bubble: #e5e7eb;    /* 机器人消息气泡 */
}

/* 示例：自定义聊天按钮大小 */
#rig-chat-button {
    width: 56px !important;
    height: 56px !important;
}
``` 