#!/usr/bin/env node
const fs = require('fs');
const path = require('path');
const { minify } = require('terser');

const JS_SOURCE_DIR = 'frontend/js';
const JS_OUTPUT_DIR = 'static/js';
const HTML_SOURCE_DIR = 'frontend';
const HTML_OUTPUT_DIR = 'static';

// HTML 压缩函数
function compressHtml(html) {
    const placeholders = [];
    let compressed = html;
    
    // 1. 保护 <script> 和 <style> 标签内的内容
    compressed = compressed.replace(/<script[^>]*>([\s\S]*?)<\/script>/gi, (match) => {
        const placeholder = `__SCRIPT_${placeholders.length}__`;
        placeholders.push(match);
        return placeholder;
    });
    
    compressed = compressed.replace(/<style[^>]*>([\s\S]*?)<\/style>/gi, (match) => {
        const placeholder = `__STYLE_${placeholders.length}__`;
        placeholders.push(match);
        return placeholder;
    });
    
    // 2. 保护 <pre> 和 <textarea> 标签内的内容（需要保留格式）
    compressed = compressed.replace(/<(pre|textarea)[^>]*>[\s\S]*?<\/\1>/gi, (match) => {
        const placeholder = `__PRE_${placeholders.length}__`;
        placeholders.push(match);
        return placeholder;
    });
    
    // 3. 移除 HTML 注释
    compressed = compressed.replace(/<!--[\s\S]*?-->/g, '');
    
    // 4. 保护属性值中的空格
    const attrPlaceholders = [];
    compressed = compressed.replace(/(\w+)="([^"]*)"/g, (match, attrName, attrValue) => {
        if (attrValue.trim()) {
            const placeholder = `${attrName}="__ATTR_${attrPlaceholders.length}__"`;
            attrPlaceholders.push(attrValue);
            return placeholder;
        }
        return match;
    });
    
    // 5. 压缩 HTML
    compressed = compressed
        .replace(/>\s+</g, '><')           // 移除标签之间的空格
        .replace(/\s{2,}/g, ' ')           // 多个空格压缩为一个
        .replace(/\n\s*/g, '')             // 移除换行和缩进
        .trim();
    
    // 6. 恢复属性值
    attrPlaceholders.forEach((value, index) => {
        compressed = compressed.replace(`__ATTR_${index}__`, () => value);
    });
    
    // 7. 恢复被保护的标签内容，并压缩其中的 JS 和 CSS
    placeholders.forEach((content, index) => {
        let replacement = content;
        
        // 压缩 <script> 标签中的 JS
        if (content.match(/<script[^>]*>/i)) {
            replacement = content.replace(/(<script[^>]*>)([\s\S]*?)(<\/script>)/i, (match, open, code, close) => {
                if (!code.trim()) return match;
                const minified = code
                    .replace(/\/\*[\s\S]*?\*\//g, '')  // 移除多行注释
                    .replace(/\/\/.*/g, '')            // 移除单行注释
                    .replace(/\n\s*/g, '')             // 移除换行和缩进
                    .replace(/\s{2,}/g, ' ')           // 多个空格压缩为一个
                    .trim();
                return open + minified + close;
            });
        }
        
        // 压缩 <style> 标签中的 CSS
        if (content.match(/<style[^>]*>/i)) {
            replacement = content.replace(/(<style[^>]*>)([\s\S]*?)(<\/style>)/i, (match, open, css, close) => {
                if (!css.trim()) return match;
                const minified = css
                    .replace(/\/\*[\s\S]*?\*\//g, '')  // 移除注释
                    .replace(/\n\s*/g, '')             // 移除换行和缩进
                    .replace(/\s*:\s*/g, ':')          // 移除冒号周围空格
                    .replace(/\s*;\s*/g, ';')          // 移除分号周围空格
                    .replace(/\s*\{\s*/g, '{')         // 移除大括号周围空格
                    .replace(/\s*\}\s*/g, '}')
                    .replace(/\s{2,}/g, ' ')           // 多个空格压缩为一个
                    .trim();
                return open + minified + close;
            });
        }
        
        const scriptPlaceholder = `__SCRIPT_${index}__`;
        const stylePlaceholder = `__STYLE_${index}__`;
        const prePlaceholder = `__PRE_${index}__`;
        
        if (compressed.includes(scriptPlaceholder)) {
            compressed = compressed.replace(scriptPlaceholder, () => replacement);
        } else if (compressed.includes(stylePlaceholder)) {
            compressed = compressed.replace(stylePlaceholder, () => replacement);
        } else if (compressed.includes(prePlaceholder)) {
            compressed = compressed.replace(prePlaceholder, () => replacement);
        }
    });
    
    return compressed;
}

// 统一自动压缩所有模板字符串中的 HTML/CSS 内容
function compressTemplateStrings(code) {
    return code.replace(/`([^`]*)`/g, (match, content) => {
        if (!content.trim()) {
            return match;
        }
        
        // 检测是否包含 HTML 标签
        const hasHTMLTags = /<[a-zA-Z][^>]*>/.test(content);
        // 检测是否包含 CSS 属性（key: value; 或 selector { } 格式）
        const hasCSSProps = /([\w-]+\s*:\s*[^:;]+;)|([.#]?[\w-]+\s*\{)/.test(content);
        
        // 如果既不是 HTML 也不是 CSS，保持原样
        if (!hasHTMLTags && !hasCSSProps) {
            return match;
        }
        
        let compressed = content;
        
        // 移除 HTML 注释 <!-- -->
        if (hasHTMLTags) {
            compressed = compressed.replace(/<!--[\s\S]*?-->/g, '');
        }
        
        // 移除 CSS 注释 /* */
        if (hasCSSProps) {
            compressed = compressed.replace(/\/\*[\s\S]*?\*\//g, '');
        }
        
        // 压缩 HTML - 移除标签之间的多余空格，但保留属性值内的空格
        if (hasHTMLTags) {
            const attrPlaceholders = [];
            
            // 匹配属性值，支持包含 ${} 的模板字符串
            compressed = compressed.replace(/(\w+)="([^"]*)"/g, (match, attrName, attrValue) => {
                // 如果属性值包含空格，保护它
                if (attrValue.includes(' ') || attrValue.includes('${')) {
                    const placeholder = `__ATTR_${attrPlaceholders.length}__`;
                    attrPlaceholders.push(attrValue);
                    return `${attrName}="${placeholder}"`;
                }
                return match;
            });
            
            // 移除标签之间的空格
            compressed = compressed
                .replace(/>\s+</g, '><');       // 移除标签之间的空格
            
            // 恢复属性值（使用函数避免 $ 的特殊处理）
            attrPlaceholders.forEach((value, index) => {
                compressed = compressed.replace(`__ATTR_${index}__`, () => value);
            });
            
            // 最后再处理其他多余空格（不在属性值内的）
            compressed = compressed.replace(/\s{2,}/g, ' ');
        }
        
        // 压缩 CSS - 移除格式化空格
        if (hasCSSProps) {
            compressed = compressed
                .replace(/\s*:\s*/g, ':')       // 移除冒号周围空格
                .replace(/\s*;\s*/g, ';')       // 移除分号周围空格
                .replace(/\s*\{\s*/g, '{')      // 移除大括号周围空格
                .replace(/\s*\}\s*/g, '}');     // 移除大括号周围空格
        }
        
        // 通用压缩：移除换行和缩进
        compressed = compressed
            .replace(/\n\s*/g, '')              // 移除换行和缩进
            .replace(/;\s*$/g, '')              // 移除末尾分号
            .trim();                            // 移除首尾空格
        
        return `\`${compressed}\``;
    });
}

async function minifyJsFile(filename) {
    const inputPath = path.join(JS_SOURCE_DIR, filename);
    const outputPath = path.join(JS_OUTPUT_DIR, filename);
    
    console.log(`🔨 Minifying JS: ${filename}...`);
    
    let code = fs.readFileSync(inputPath, 'utf8');
    const originalSize = code.length;
    
    // 先自动压缩所有模板字符串中的 HTML/CSS
    code = compressTemplateStrings(code);
    const afterStringCompressionSize = code.length;
    
    const result = await minify(code, {
        compress: {
            dead_code: false,
            drop_console: false,      
            drop_debugger: true,      // 移除debugger
            keep_classnames: false,
            keep_fargs: false,
            keep_fnames: false,
            keep_infinity: false,
            passes: 1,                // 压缩优化次数
        },
        mangle: {
            keep_classnames: false,
            keep_fnames: false,
        },
        format: {
            comments: false,          // 移除所有注释
            ascii_only: false,
        },
        sourceMap: false,
    });
    
    if (result.error) {
        console.error(`❌ Error minifying ${filename}:`, result.error);
        process.exit(1);
    }
    
    fs.writeFileSync(outputPath, result.code);
    
    const minifiedSize = result.code.length;
    const stringSavings = afterStringCompressionSize < originalSize ? 
        ` [Strings: -${formatSize(originalSize - afterStringCompressionSize)}]` : '';
    const totalSavings = ((1 - minifiedSize / originalSize) * 100).toFixed(1);
    
    console.log(`✅ ${filename}: ${formatSize(originalSize)} → ${formatSize(minifiedSize)} (${totalSavings}% smaller)${stringSavings}`);
}

async function minifyHtmlFile(filename) {
    const inputPath = path.join(HTML_SOURCE_DIR, filename);
    const outputPath = path.join(HTML_OUTPUT_DIR, filename);
    
    console.log(`🔨 Minifying HTML: ${filename}...`);
    
    const html = fs.readFileSync(inputPath, 'utf8');
    const originalSize = html.length;
    
    // 使用自定义的 HTML 压缩函数
    const minified = compressHtml(html);
    
    fs.writeFileSync(outputPath, minified);
    
    const minifiedSize = minified.length;
    const totalSavings = ((1 - minifiedSize / originalSize) * 100).toFixed(1);
    
    console.log(`✅ ${filename}: ${formatSize(originalSize)} → ${formatSize(minifiedSize)} (${totalSavings}% smaller)`);
}

function formatSize(bytes) {
    if (bytes < 1024) return bytes + 'B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + 'KB';
    return (bytes / (1024 * 1024)).toFixed(1) + 'MB';
}

async function build() {
    console.log('🚀 Building frontend with Terser...\n');
    
    // 确保输出目录存在
    if (!fs.existsSync(JS_OUTPUT_DIR)) {
        fs.mkdirSync(JS_OUTPUT_DIR, { recursive: true });
    }
    if (!fs.existsSync(HTML_OUTPUT_DIR)) {
        fs.mkdirSync(HTML_OUTPUT_DIR, { recursive: true });
    }
    
    // 获取所有JS文件
    const jsFiles = fs.readdirSync(JS_SOURCE_DIR).filter(f => f.endsWith('.js'));
    
    // 压缩所有JS文件
    console.log('📦 Processing JavaScript files...');
    for (const file of jsFiles) {
        await minifyJsFile(file);
    }
    
    // 获取所有HTML文件
    const htmlFiles = fs.readdirSync(HTML_SOURCE_DIR).filter(f => f.endsWith('.html'));
    
    // 压缩所有HTML文件
    if (htmlFiles.length > 0) {
        console.log('\n📄 Processing HTML files...');
        for (const file of htmlFiles) {
            await minifyHtmlFile(file);
        }
    }
    
    console.log('\n✨ Build complete!');
}

build().catch(error => {
    console.error('❌ Build failed:', error);
    process.exit(1);
});

