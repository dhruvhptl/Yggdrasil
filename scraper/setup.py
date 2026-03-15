"""
Run once after `pip install -r requirements.txt` to download Scrapling browsers.
This downloads ~500MB of browser binaries needed for StealthyFetcher and DynamicFetcher.
"""
import subprocess
import sys

print("Installing Scrapling browser binaries (~500MB)...")
result = subprocess.run(["scrapling", "install"], check=True)
if result.returncode == 0:
    print("✅ Scrapling browsers installed successfully.")
    print("   You can now start the scraper: python main.py")
else:
    print("❌ Browser installation failed.")
    sys.exit(1)
