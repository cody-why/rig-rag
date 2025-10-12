use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=frontend/");

    // 检查是否跳过前端构建
    if std::env::var("SKIP_FRONTEND_BUILD").is_ok() {
        println!("Skipping frontend build");
        return;
    }

    // 检查npm
    if Command::new("npm").arg("--version").output().is_err() {
        eprintln!("Warning: npm not found. Run 'npm install && npm run build' manually.");
        return;
    }

    // 构建前端
    println!("🚀 Building frontend with esbuild...");
    let status = Command::new("npm")
        .args(["run", "build"])
        .status()
        .expect("Failed to run build");

    if status.success() {
        println!("✅ Frontend build complete!");
    } else {
        eprintln!("❌ Frontend build failed");
    }
}
