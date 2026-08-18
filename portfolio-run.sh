# Full deployment script for portfolio project
# This script builds project and serves on cloudflare tunnel

rustup update

cd portfolio-serve
cargo build --release
cd ..

mv portfolio-serve/target/release/portfolio-serve portfolio-serve

chmod +x portfolio-serve

./portfolio-serve

# cloudflare handles url when i add that
cloudflared tunnel run portfolio-tunnel