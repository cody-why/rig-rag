
class RigChat {
    constructor(config = {}) {
        this.config = {
            apiBase: config.apiBase || '', // 默认使用当前域名
            theme: config.theme || 'light', // light 或 dark
            position: config.position || 'right', // right 或 left
            welcomeMessage: config.welcomeMessage || '你好，我是AI Assistant，很高兴为您服务！',
            buttonIcon: config.buttonIcon || 'ai', // 图标名称
            title: config.title || 'AI Assistant',
            placeholder: config.placeholder || 'Type your message...',
            containerId: config.containerId || 'rig-chat-container',
            defaultWidth: config.defaultWidth || 450,
            defaultHeight: config.defaultHeight || 550
        };
        
        this.init();
    }
    
    async init() {
        console.log('Initializing Rig Chat component');
        
        // 加载样式和依赖
        this.injectStyles();
        this.loadFontAwesome();
        await this.loadMarked();
        
        // 从本地存储还原主题
        const savedTheme = localStorage.getItem('rig_chat_theme');
        if (savedTheme) {
            this.config.theme = savedTheme;
        }
        
        // 从本地存储还原宽高
        const savedWidth = localStorage.getItem('rig_chat_width');
        if (savedWidth) {
            this.config.defaultWidth = parseInt(savedWidth);
        }
        const savedHeight = localStorage.getItem('rig_chat_height');
        if (savedHeight) {
            this.config.defaultHeight = parseInt(savedHeight);
        }
        
        // 创建UI并初始化事件
        this.createChatWidget();
        this.initEventListeners();
        
        // 加载历史消息
        await this.loadChatHistory();
        
        // 应用尺寸
        this.applyDimensions();
        
        console.log('Rig Chat component initialized');
    }

    // 创建样式
    injectStyles() {
        const styleEl = document.createElement('style');
        styleEl.textContent = `
        /* Rig Chat 基础样式 */
        #rig-chat-container * {
            box-sizing: border-box;
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Oxygen, Ubuntu, Cantarell, "Open Sans", "Helvetica Neue", sans-serif;
        }
        
        /* 聊天容器 */
        .chat-container {
            height: calc(100vh - 180px);
            max-height: 500px;
        }
        
        .chat-widget {
            transition: all 0.3s ease;
            transform-origin: bottom right;
            /* 移除固定宽高，通过JS动态设置 */
            max-width: none !important; /* 移除最大宽度限制 */
            position: relative;
            min-width: 300px;
            min-height: 400px;
            overflow: hidden;
        }
        
        .chat-widget.collapsed {
            transform: scale(0);
            opacity: 0;
            pointer-events: none;
        }
        
        .chat-widget.expanded {
            transition: all 0.3s ease;
        }
        
        .chat-button {
            box-shadow: 0 4px 15px rgba(0, 0, 0, 0.15);
            transition: transform 0.2s;
            cursor: pointer;
        }
        
        .chat-button:hover {
            transform: scale(1.05);
        }
        
        /* 拖拽调整大小手柄 */
        .resize-handle {
            position: absolute;
            top: 1px;
            left: 1px;
            width: 20px;
            height: 20px;
            background-color: var(--bg-accent);
            opacity: 0.5;
            z-index: 999;
            cursor: nwse-resize;
            text-align: center;
            font-size: 14px;
            color: white;
        }
        
        .resize-handle:hover {
            opacity: 0.8;
        }
        
        .resize-handle::before {
            content: "⋮⋮";
            display: block;
        }
        
        /* 头部控制栏 */
        .header-controls {
            display: flex;
            align-items: center;
            gap: 8px;
        }
        
        /* 调整标题文字，给左侧手柄留出空间 */
        .rig-header-title {
            margin-left: 10px;
        }
        
        /* 按钮样式 */
        .rig-btn {
            background: none;
            border: none;
            color: var(--text-accent);
            opacity: 0.8;
            cursor: pointer;
            padding: 0;
            font-size: 16px;
            width: 28px;
            height: 28px;
            display: flex;
            align-items: center;
            justify-content: center;
            border-radius: 4px;
        }
        
        .rig-btn:hover {
            opacity: 1;
            background-color: rgba(255, 255, 255, 0.1);
        }
        
        #rig-chat-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            background-color: var(--bg-accent);
            color: var(--text-accent);
            padding: 12px 15px;
            border-top-left-radius: 10px;
            border-top-right-radius: 10px;
        }
        
        /* 消息动画 */
        @keyframes slideIn {
            from {
                opacity: 0;
                transform: translateY(10px);
            }
            to {
                opacity: 1;
                transform: translateY(0);
            }
        }
        
        .message-animation {
            animation: slideIn 0.3s ease forwards;
        }
        
        /* 加载指示器 */
        .typing-indicator {
            display: inline-block;
        }
        .typing-indicator span {
            display: inline-block;
            width: 5px;
            height: 5px;
            background-color: currentColor;
            border-radius: 50%;
            margin: 0 1px;
            animation: bounce 1.2s infinite;
        }
        .typing-indicator span:nth-child(2) {
            animation-delay: 0.2s;
        }
        .typing-indicator span:nth-child(3) {
            animation-delay: 0.4s;
        }
        @keyframes bounce {
            0%, 60%, 100% { transform: translateY(0); }
            30% { transform: translateY(-4px); }
        }
        
        /* 滚动条样式 */
        .custom-scrollbar::-webkit-scrollbar {
            width: 6px;
        }
        .custom-scrollbar::-webkit-scrollbar-track {
            background: rgba(255, 255, 255, 0.5);
            border-radius: 10px;
        }
        .custom-scrollbar::-webkit-scrollbar-thumb {
            background: rgba(0, 0, 0, 0.2);
            border-radius: 10px;
        }
        .custom-scrollbar::-webkit-scrollbar-thumb:hover {
            background: rgba(0, 0, 0, 0.5);
        }
        
        .theme-dark .custom-scrollbar::-webkit-scrollbar-track {
            background: rgba(0, 0, 0, 0.5);
        }
        .theme-dark .custom-scrollbar::-webkit-scrollbar-thumb {
            background: rgba(255, 255, 255, 0.5);
        }
        .theme-dark .custom-scrollbar::-webkit-scrollbar-thumb:hover {
            background: rgba(255, 255, 255, 0.6);
        }
        
        /* 主题色 */
        .theme-light {
            --bg-primary: #ffffff;
            --bg-secondary: #f3f4f6;
            --bg-accent: #0ea5e9;
            --text-primary: #1f2937;
            --text-secondary: #6b7280;
            --text-accent: #ffffff;
            --border-color: #e5e7eb;
            --user-bubble: #0ea5e9;
            --bot-bubble: #e5e7eb;
            --user-text: #ffffff;
            --bot-text: #1f2937;
        }
        
        .theme-dark {
            --bg-primary: #1f2937;
            --bg-secondary: #111827;
            --bg-accent: #1e293b;
            --text-primary: #f9fafb;
            --text-secondary: #9ca3af;
            --text-accent: #ffffff;
            --border-color: #374151;
            --user-bubble: #3b82f6;
            --bot-bubble: #374151;
            --user-text: #ffffff;
            --bot-text: #e5e7eb;
        }
        
        /* 聊天组件样式 */
        #rig-chat-button {
            position: fixed;
            bottom: 20px;
            width: 46px;
            height: 46px;
            border-radius: 50%;
            display: flex;
            align-items: center;
            justify-content: center;
            z-index: 9999;
            color: var(--text-accent);
            background-color: var(--bg-accent);
            border: none;
        }
        #rig-chat-button.position-right { right: 20px; }
        #rig-chat-button.position-left { left: 20px; }
        
        #rig-chat-widget {
            position: fixed;
            bottom: 80px;
            width: 100%;
            max-width: 450px;
            height: 550px;
            border-radius: 10px;
            overflow: hidden;
            background-color: var(--bg-primary);
            box-shadow: 0 4px 25px rgba(0, 0, 0, 0.1);
            z-index: 9998;
            display: flex;
            flex-direction: column;
        }
        #rig-chat-widget.position-right { right: 20px; }
        #rig-chat-widget.position-left { left: 20px; }
        
        #rig-chat-messages {
            flex: 1;
            overflow-y: auto;
            padding: 15px;
            background-color: var(--bg-secondary);
            display: flex;
            flex-direction: column;
            gap: 10px;
        }
        
        #rig-chat-input-container {
            padding: 10px 15px;
            border-top: 1px solid var(--border-color);
            background-color: var(--bg-primary);
            position: relative;
        }
        
        #rig-chat-input {
            width: 100%;
            padding: 12px 40px 12px 12px;
            border-radius: 8px;
            border: 1px solid var(--border-color);
            background-color: var(--bg-secondary);
            color: var(--text-primary);
            outline: none;
        }
        
        #rig-chat-send {
            position: absolute;
            right: 25px;
            top: 50%;
            transform: translateY(-50%);
            background: none;
            border: none;
            color: var(--bg-accent);
            cursor: pointer;
        }
        
        /* 为暗黑模式添加特定的发送按钮颜色 */
        .theme-dark #rig-chat-send {
            color: #3b82f6;
        }
        
        .rig-message-bubble {
            max-width: 92%;
            padding: 12px 15px;
            border-radius: 10px;
            margin-bottom: 8px;
            word-break: break-word;
        }
        
        .rig-user-message {
            align-self: flex-end;
            background-color: var(--user-bubble);
            color: var(--user-text);
        }
        
        .rig-bot-message {
            align-self: flex-start;
            background-color: var(--bot-bubble);
            color: var(--bot-text);
            width: 96%;
        }
        
        /* 格式化内容样式 */
        .rig-bot-message code {
            font-family: monospace;
            background-color: rgba(0, 0, 0, 0.1);
            padding: 2px 4px;
            border-radius: 4px;
            font-size: 0.9em;
        }
        
        .rig-bot-message pre {
            background-color: #282c34;
            color: #abb2bf;
            padding: 12px;
            border-radius: 6px;
            overflow-x: auto;
            margin: 10px 0;
        }
        
        .rig-bot-message table {
            border-collapse: collapse;
            width: 100%;
            margin: 10px 0;
            border-radius: 8px;
            overflow: hidden;
            box-shadow: 0 2px 5px rgba(168, 168, 168, 0.05);
            table-layout: fixed;
            font-size: 0.92em;
            border: 1px solid rgba(255, 255, 255, 0.5);
        }
        
        .rig-bot-message th, .rig-bot-message td {
            border: 1px solid rgba(255, 255, 255, 0.5);
            padding: 8px 10px;
            text-align: center;
            overflow: visible;
            white-space: normal;
            word-wrap: break-word;
        }
        
        .rig-bot-message th {
            background-color: var(--bg-accent);
            color: var(--text-accent);
            font-weight: 600;
            border-bottom: 2px solid var(--border-color);
            position: relative;
        }
        
        .rig-bot-message tr:nth-child(even) {
            background-color: rgba(0, 0, 0, 0.03);
        }
        
        .rig-bot-message tr:hover {
            background-color: rgba(0, 0, 0, 0.06);
        }
        
        .theme-dark .rig-bot-message th {
            background-color: var(--bg-accent);
            color: var(--text-accent);
        }
        
        .theme-dark .rig-bot-message tr:nth-child(even) {
            background-color: rgba(255, 255, 255, 0.03);
        }
        
        .theme-dark .rig-bot-message tr:hover {
            background-color: rgba(255, 255, 255, 0.05);
        }
        
        .theme-dark .rig-bot-message table {
            border: 1px solid rgba(180, 180, 180, 0.2);
        }
        
        .theme-dark .rig-bot-message th, .theme-dark .rig-bot-message td {
            border: 1px solid rgba(180, 180, 180, 0.15);
        }
        
        .rig-bot-message ul, .rig-bot-message ol {
            margin: 10px 0;
            padding-left: 20px;
        }
        
        .rig-bot-message ul li {
            list-style-type: disc;
        }
        
        .rig-bot-message ol li {
            list-style-type: decimal;
        }
        `;
        document.head.appendChild(styleEl);
    }

    // 加载 Font Awesome
    loadFontAwesome() {
 
        // 创建SVG图标库
        const svgIcons = {
            'ai': '<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 24 24"><g fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" color="currentColor"><path d="M14.17 20.89c4.184-.277 7.516-3.657 7.79-7.9c.053-.83.053-1.69 0-2.52c-.274-4.242-3.606-7.62-7.79-7.899a33 33 0 0 0-4.34 0c-4.184.278-7.516 3.657-7.79 7.9a20 20 0 0 0 0 2.52c.1 1.545.783 2.976 1.588 4.184c.467.845.159 1.9-.328 2.823c-.35.665-.526.997-.385 1.237c.14.24.455.248 1.084.263c1.245.03 2.084-.322 2.75-.813c.377-.279.566-.418.696-.434s.387.09.899.3c.46.19.995.307 1.485.34c1.425.094 2.914.094 4.342 0"/><path d="m7.5 15l1.842-5.526a.694.694 0 0 1 1.316 0L12.5 15m3-6v6m-7-2h3"/></g></svg>',
            'robot': '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M512 240c0 114.9-114.6 208-256 208c-37.1 0-72.3-6.4-104.1-17.9c-11.9 8.7-31.3 20.6-54.3 30.6C73.6 471.1 44.7 480 16 480c-6.5 0-12.3-3.9-14.8-9.9c-2.5-6-1.1-12.8 3.4-17.4l0 0 0 0 0 0 0 0 .3-.3c.3-.3 .7-.7 1.3-1.4c1.1-1.2 2.8-3.1 4.9-5.7c4.1-5 9.6-12.4 15.2-21.6c10-16.6 19.5-38.4 21.4-62.9C17.7 326.8 0 285.1 0 240C0 125.1 114.6 32 256 32s256 93.1 256 208z"/></svg>',
            'times': '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 384 512"><path d="M342.6 150.6c12.5-12.5 12.5-32.8 0-45.3s-32.8-12.5-45.3 0L192 210.7 86.6 105.4c-12.5-12.5-32.8-12.5-45.3 0s-12.5 32.8 0 45.3L146.7 256 41.4 361.4c-12.5 12.5-12.5 32.8 0 45.3s32.8 12.5 45.3 0L192 301.3 297.4 406.6c12.5 12.5 32.8 12.5 45.3 0s12.5-32.8 0-45.3L237.3 256 342.6 150.6z"/></svg>',
            'sun': '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M361.5 1.2c5 2.1 8.6 6.6 9.6 11.9L391 121l107.9 19.8c5.3 1 9.8 4.6 11.9 9.6s1.5 10.7-1.6 15.2L446.9 256l62.3 90.3c3.1 4.5 3.7 10.2 1.6 15.2s-6.6 8.6-11.9 9.6L391 391 371.1 498.9c-1 5.3-4.6 9.8-9.6 11.9s-10.7 1.5-15.2-1.6L256 446.9l-90.3 62.3c-4.5 3.1-10.2 3.7-15.2 1.6s-8.6-6.6-9.6-11.9L121 391 13.1 371.1c-5.3-1-9.8-4.6-11.9-9.6s-1.5-10.7 1.6-15.2L65.1 256 2.8 165.7c-3.1-4.5-3.7-10.2-1.6-15.2s6.6-8.6 11.9-9.6L121 121 140.9 13.1c1-5.3 4.6-9.8 9.6-11.9s10.7-1.5 15.2 1.6L256 65.1 346.3 2.8c4.5-3.1 10.2-3.7 15.2-1.6zM160 256a96 96 0 1 1 192 0 96 96 0 1 1 -192 0zm224 0a128 128 0 1 0 -256 0 128 128 0 1 0 256 0z"/></svg>',
            'moon': '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 384 512"><path d="M223.5 32C100 32 0 132.3 0 256S100 480 223.5 480c60.6 0 115.5-24.2 155.8-63.4c5-4.9 6.3-12.5 3.1-18.7s-10.1-9.7-17-8.5c-9.8 1.7-19.8 2.6-30.1 2.6c-96.9 0-175.5-78.8-175.5-176c0-65.8 36-123.1 89.3-153.3c6.1-3.5 9.2-10.5 7.7-17.3s-7.3-11.9-14.3-12.5c-6.3-.5-12.6-.8-19-.8z"/></svg>',
            'paper-plane': '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><path d="M498.1 5.6c10.1 7 15.4 19.1 13.5 31.2l-64 416c-1.5 9.7-7.4 18.2-16 23s-18.9 5.4-28 1.6L284 427.7l-68.5 74.1c-8.9 9.7-22.9 12.9-35.2 8.1S160 493.2 160 480V396.4c0-4 1.5-7.8 4.2-10.7L331.8 202.8c5.8-6.3 5.6-16-.4-22s-15.7-6.4-22-.7L106 360.8 17.7 316.6C7.1 311.3 .3 300.7 0 288.9s5.9-22.8 16.1-28.7l448-256c10.7-6.1 23.9-5.5 34 1.4z"/></svg>',
            'expand': '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 448 512"><path d="M32 32C14.3 32 0 46.3 0 64v96c0 17.7 14.3 32 32 32s32-14.3 32-32V96h64c17.7 0 32-14.3 32-32s-14.3-32-32-32H32zM64 352c0-17.7-14.3-32-32-32s-32 14.3-32 32v96c0 17.7 14.3 32 32 32h96c17.7 0 32-14.3 32-32s-14.3-32-32-32H64V352zM320 32c-17.7 0-32 14.3-32 32s14.3 32 32 32h64v64c0 17.7 14.3 32 32 32s32-14.3 32-32V64c0-17.7-14.3-32-32-32H320zM448 352c0-17.7-14.3-32-32-32s-32 14.3-32 32v64H320c-17.7 0-32 14.3-32 32s14.3 32 32 32h96c17.7 0 32-14.3 32-32V352z"/></svg>',
        };
        
        // 添加到全局对象
        window.svgIcons = svgIcons;
    }

    // 加载 Marked (Markdown 解析)
    loadMarked() {
        return new Promise((resolve) => {
            if (window.marked) {
                resolve();
                return;
            }
            
            const script = document.createElement('script');
            script.src = 'https://cdn.jsdelivr.net/npm/marked@4.3.0/marked.min.js';
            script.onload = () => {
                console.log('Marked library loaded');
                resolve();
            };
            script.onerror = (e) => {
                console.error('Error loading Marked library:', e);
                resolve(); // 继续执行，即使加载失败
            };
            document.head.appendChild(script);
        });
    }

    // 创建聊天组件
    createChatWidget() {
        // 创建容器
        const container = document.createElement('div');
        container.id = this.config.containerId;
        document.body.appendChild(container);
        
        // 使用SVG图标
        const getSvgIcon = (name) => window.svgIcons[name] || window.svgIcons['comment'];
        
        const buttonIcon = getSvgIcon(this.config.buttonIcon);
        
        container.innerHTML = `
        <button id="rig-chat-button" class="chat-button position-${this.config.position} theme-${this.config.theme}" data-icon="${this.config.buttonIcon}">
            ${buttonIcon}
        </button>
        
        <div id="rig-chat-widget" class="chat-widget collapsed position-${this.config.position} theme-${this.config.theme}" style="width:${this.config.defaultWidth}px;height:${this.config.defaultHeight}px;">
            <div id="rig-chat-header">
                <div class="rig-header-title">${this.config.title}</div>
                <div class="header-controls">
                    <button id="rig-theme-toggle" class="rig-btn" title="切换主题">
                        ${getSvgIcon(this.config.theme === 'dark' ? 'sun' : 'moon')}
                    </button>
                    <button id="rig-minimize" class="rig-btn" title="关闭">
                        ${getSvgIcon('times')}
                    </button>
                </div>
            </div>
            
            <div class="resize-handle" id="rig-resize-handle" title="拖动调整大小"></div>
            
            <div id="rig-chat-messages" class="custom-scrollbar">
                <div class="rig-message-bubble rig-bot-message message-animation">
                    ${this.config.welcomeMessage}
                </div>
            </div>
            
            <div id="rig-chat-input-container">
                <input id="rig-chat-input" type="text" placeholder="${this.config.placeholder}">
                <button id="rig-chat-send">
                    ${getSvgIcon('paper-plane')}
                </button>
            </div>
        </div>
        `;
        
        // 应用当前主题
        document.documentElement.classList.add(`theme-${this.config.theme}`);
        
        // 添加图标样式
        const buttonStyle = document.createElement('style');
        buttonStyle.textContent = `
            #rig-chat-button svg {
                width: 24px;
                height: 24px;
                fill: currentColor;
            }
            #rig-chat-button[data-icon="robot"] svg {
                width: 32px;
                height: 32px;
            }
            #rig-chat-widget button svg {
                width: 16px;
                height: 16px;
                fill: currentColor;
            }
        `;
        document.head.appendChild(buttonStyle);
    }

    // 初始化事件监听
    initEventListeners() {
        const widget = document.getElementById('rig-chat-widget');
        const button = document.getElementById('rig-chat-button');
        const minimize = document.getElementById('rig-minimize');
        const themeToggle = document.getElementById('rig-theme-toggle');
        const input = document.getElementById('rig-chat-input');
        const sendButton = document.getElementById('rig-chat-send');
        const resizeHandle = document.getElementById('rig-resize-handle');
        
        // 显示/隐藏聊天窗口
        button.addEventListener('click', () => {
            widget.classList.toggle('collapsed');
            const newIcon = widget.classList.contains('collapsed') ? this.config.buttonIcon : 'times';
            button.setAttribute('data-icon', newIcon);
            button.innerHTML = window.svgIcons[newIcon];
        });
        
        // 最小化聊天窗口
        minimize.addEventListener('click', () => {
            widget.classList.add('collapsed');
            button.setAttribute('data-icon', this.config.buttonIcon);
            button.innerHTML = window.svgIcons[this.config.buttonIcon];
        });
        
        // 切换主题
        themeToggle.addEventListener('click', () => {
            const isLight = widget.classList.contains('theme-light');
            const newTheme = isLight ? 'dark' : 'light';
            
            // 更新聊天窗口主题
            widget.classList.remove(`theme-${isLight ? 'light' : 'dark'}`);
            widget.classList.add(`theme-${newTheme}`);
            
            // 更新浮动按钮主题
            button.classList.remove(`theme-${isLight ? 'light' : 'dark'}`);
            button.classList.add(`theme-${newTheme}`);
            
            // 更新根元素主题类，确保滚动条等全局样式正确应用
            document.documentElement.classList.remove(`theme-${isLight ? 'light' : 'dark'}`);
            document.documentElement.classList.add(`theme-${newTheme}`);
            
            // 更新按钮图标
            themeToggle.innerHTML = window.svgIcons[isLight ? 'sun' : 'moon'];
            
            // 保存主题设置
            this.config.theme = newTheme;
            localStorage.setItem('rig_chat_theme', newTheme);
        });
        
        // 处理鼠标拖动调整大小
        if (resizeHandle) {
            let isResizing = false;
            let lastX, lastY, initialWidth, initialHeight, initialTop, initialLeft;
            let isRightSide = this.config.position === 'right';
            
            // 开始拖动
            resizeHandle.addEventListener('mousedown', (e) => {
                isResizing = true;
                lastX = e.clientX;
                lastY = e.clientY;
                initialWidth = widget.offsetWidth;
                initialHeight = widget.offsetHeight;
                initialTop = widget.getBoundingClientRect().top;
                initialLeft = widget.getBoundingClientRect().left;
                
                // 添加临时事件监听器
                document.addEventListener('mousemove', handleMouseMove);
                document.addEventListener('mouseup', handleMouseUp);
                
                // 防止事件传播和文本选择
                e.preventDefault();
                e.stopPropagation();
            });
            
            // 处理鼠标移动
            const handleMouseMove = (e) => {
                if (!isResizing) return;
                
                const deltaX = e.clientX - lastX;
                const deltaY = e.clientY - lastY;
                
                // 获取浏览器视口宽高
                const viewportWidth = window.innerWidth;
                const viewportHeight = window.innerHeight;
                
                // 获取当前窗口的位置和尺寸
                const rect = widget.getBoundingClientRect();
                
                // 边界安全距离
                const safeMargin = 10;
                
                // 计算新的尺寸和位置
                let newWidth, newHeight;
                
                if (isRightSide) {
                    // 右侧模式 - 向左拖动增加宽度（负deltaX表示向左拖动）
                    const maxWidth = viewportWidth - safeMargin - (viewportWidth - rect.right);
                    newWidth = Math.min(maxWidth, Math.max(300, initialWidth - deltaX));
                    
                    // 向上拖动增加高度（负deltaY表示向上拖动）
                    const maxHeight = viewportHeight - safeMargin - (viewportHeight - rect.bottom);
                    newHeight = Math.min(maxHeight, Math.max(400, initialHeight - deltaY));
                    
                    // 保持右侧位置固定
                    widget.style.right = widget.style.right || '20px';
                } else {
                    // 左侧模式
                    const maxWidth = viewportWidth - safeMargin - rect.left;
                    newWidth = Math.min(maxWidth, Math.max(300, initialWidth + deltaX));
                    
                    const maxHeight = viewportHeight - safeMargin - (viewportHeight - rect.bottom);
                    newHeight = Math.min(maxHeight, Math.max(400, initialHeight - deltaY));
                    
                    // 保持左侧位置固定
                    widget.style.left = widget.style.left || '20px';
                }
                
                // 应用新尺寸
                widget.style.width = `${newWidth}px`;
                widget.style.height = `${newHeight}px`;
                
                // 更新消息区域大小
                const messagesArea = document.getElementById('rig-chat-messages');
                if (messagesArea) {
                    messagesArea.style.maxHeight = `${newHeight - 100}px`;
                }
                
                // 保存实际宽高
                this.config.defaultWidth = newWidth;
                this.config.defaultHeight = newHeight;
                localStorage.setItem('rig_chat_width', newWidth.toString());
                localStorage.setItem('rig_chat_height', newHeight.toString());
                
                // 更新扩展状态
                widget.classList.toggle('expanded', newWidth > 450 || newHeight > 550);
                
                // 防止文本选择
                e.preventDefault();
            };
            
            // 结束拖动
            const handleMouseUp = () => {
                isResizing = false;
                document.removeEventListener('mousemove', handleMouseMove);
                document.removeEventListener('mouseup', handleMouseUp);
            };
        }
        
        // 处理输入
        input.addEventListener('keypress', (e) => {
            if (e.key === 'Enter') {
                this.sendMessage();
            }
        });
        
        sendButton.addEventListener('click', this.sendMessage.bind(this));
    }

    // 应用尺寸
    applyDimensions() {
        const widget = document.getElementById('rig-chat-widget');
        if (!widget) return; // 确保元素存在
        
        const width = this.config.defaultWidth;
        const height = this.config.defaultHeight;
        
        // 应用尺寸
        widget.style.width = `${width}px`;
        widget.style.height = `${height}px`;
        
        // 添加扩展标记类
        widget.classList.toggle('expanded', width > 450 || height > 550);
        
        // 确保消息区域适应新的尺寸
        const messagesArea = document.getElementById('rig-chat-messages');
        if (messagesArea) {
            messagesArea.style.maxHeight = `${height - 100}px`;
        }
    }

    // 发送消息
    async sendMessage() {
        const input = document.getElementById('rig-chat-input');
        const message = input.value.trim();
        if (!message) return;
        
        // 添加用户消息
        this.addMessage(message, true);
        input.value = '';
        
        // 添加加载指示器
        const loadingId = this.addLoadingIndicator();
        
        try {
            // 从本地存储获取用户ID
            let userId = localStorage.getItem('rig_chat_user_id');
            
            // 构建API URL
            const apiUrl = `${this.config.apiBase}/api/chat`;
            
            // 发送请求
            const response = await fetch(apiUrl, {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                },
                body: JSON.stringify({ 
                    message,
                    user_id: userId
                }),
            });
            
            const data = await response.json();
            
            // 如果是新用户，保存用户ID
            if (!userId && data.user_id) {
                userId = data.user_id;
                localStorage.setItem('rig_chat_user_id', userId);
            }
            
            // 移除加载指示器
            this.removeLoadingIndicator(loadingId);
            
            // 添加响应消息
            this.addMessage(data.response, false);
        } catch (error) {
            console.error('Error:', error);
            this.removeLoadingIndicator(loadingId);
            this.addMessage('Sorry, there was an error processing your message.', false);
        }
    }

    // 添加消息到聊天窗口
    addMessage(content, isUser) {
        const messagesContainer = document.getElementById('rig-chat-messages');
        
        const bubble = document.createElement('div');
        bubble.className = `rig-message-bubble ${isUser ? 'rig-user-message' : 'rig-bot-message'} message-animation`;
        
        if (!isUser && window.marked) {
            // 检测是否包含Markdown语法
            const hasMarkdown = /([*_~`]|#{1,6}|\[[^\]]+\]\([^)]+\)|```|\|[-|]|>|^\d+\.|^\s*[-*+])/.test(content);
            
            if (hasMarkdown) {
                // 特殊处理表格，确保表格前后有空行
                content = content.replace(/(\n|^)(\|[^\n]+\|)(\n|$)/g, "\n$2\n");
                
                try {
                    bubble.innerHTML = marked.parse(content);
                } catch (e) {
                    console.error("Markdown parsing error:", e);
                    bubble.textContent = content;
                }
            } else {
                // 基本处理
                bubble.textContent = content;
            }
        } else {
            bubble.textContent = content;
        }
        
        messagesContainer.appendChild(bubble);
        messagesContainer.scrollTop = messagesContainer.scrollHeight;
    }

    // 添加加载指示器
    addLoadingIndicator() {
        const messagesContainer = document.getElementById('rig-chat-messages');
        
        const bubble = document.createElement('div');
        bubble.className = 'rig-message-bubble rig-bot-message message-animation';
        bubble.id = 'rig-loading-' + Date.now();
        
        const indicator = document.createElement('div');
        indicator.className = 'typing-indicator';
        indicator.innerHTML = '<span></span><span></span><span></span>';
        
        bubble.appendChild(indicator);
        messagesContainer.appendChild(bubble);
        messagesContainer.scrollTop = messagesContainer.scrollHeight;
        
        return bubble.id;
    }

    // 移除加载指示器
    removeLoadingIndicator(id) {
        const indicator = document.getElementById(id);
        if (indicator) {
            indicator.remove();
        }
    }

    // 加载历史消息
    async loadChatHistory() {
        const userId = localStorage.getItem('rig_chat_user_id');
        if (!userId) return;
        
        try {
            const apiUrl = `${this.config.apiBase}/api/history/${userId}`;
            const response = await fetch(apiUrl);
            const history = await response.json();
            
            if (history.length > 0) {
                // 清除欢迎消息
                document.getElementById('rig-chat-messages').innerHTML = '';
                
                // 添加历史消息
                for (const message of history) {
                    this.addMessage(message.content, message.role === 'user');
                }
            }
        } catch (error) {
            console.error('Error loading chat history:', error);
        }
    }
}

// 导出 RigChat 类
if (typeof module !== 'undefined' && module.exports) {
    module.exports = RigChat;
} else {
    window.RigChat = RigChat;
} 