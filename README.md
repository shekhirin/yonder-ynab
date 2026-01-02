# yonder-ynab

> [!NOTE]
As of January 2026, YNAB doesn't support linking a Yonder account. Yonder [seems to support](https://www.tell.money/tap-global-partners-with-tell-money-to-integrate-open-banking-gateway/) Open Banking via tell.money, but YNAB doesn't work with it. This solution is a hack that will hopefully become obsolete in the near future.

Import Yonder transactions into YNAB. Runs on Cloudflare Workers.

Two ways to use this:
1. **Telegram Bot** - Send CSV files via Telegram
2. **iOS Shortcuts Webhook** - POST CSV data directly from iOS Shortcuts

Both methods do the same thing - parse Yonder CSV exports and import them to your YNAB budget. Use whichever fits your workflow.

## Setup

### 1. YNAB Configuration

Get your YNAB credentials:

1. **API Key**:
  - Go to https://app.ynab.com/settings/developer
  - Create a "Personal Access Token"
  - Save this token

2. **Budget ID** and **Account ID**:
  - Open the YNAB account you want to import to
  - URL format: `https://app.ynab.com/{budget-id}/accounts/{account-id}`
  - Copy the budget and account UUIDs from the URL

### 2. Deploy to Cloudflare Workers

```bash
# Clone the repository
git clone https://github.com/shekhirin/yonder-ynab
cd yonder-ynab

# Install Wrangler CLI (if not already installed)
npm install -g wrangler

# Authenticate with Cloudflare
wrangler login
```

### 3. Set Environment Variables

Set these secrets in Cloudflare Workers:

```bash
# Enter your YNAB Personal Access Token
wrangler secret put YNAB_API_KEY

# Enter your budget ID (or "last-used")
wrangler secret put YNAB_BUDGET_ID

# Enter your account UUID
wrangler secret put YNAB_ACCOUNT_ID
```

**For Telegram method:**
```bash
# Enter your Telegram bot token from BotFather
wrangler secret put API_KEY
```

**For iOS Shortcuts method:**
```bash
# Enter a secure random string (generate with: openssl rand -hex 32)
wrangler secret put WEBHOOK_API_KEY
```

### 3. Deploy
```bash
wrangler deploy
```

After deployment, note your worker URL: `https://<worker-name>.<your-subdomain>.workers.dev`


## Usage: Telegram Bot

### Setup

1. Create a Telegram bot:
   - Open Telegram and message [@BotFather](https://t.me/botfather)
   - Send `/newbot` and follow the prompts
   - Save the bot token (looks like `123456789:ABCdefGHIjklMNOpqrsTUVwxyz`)

2. Set the bot token as `API_KEY` secret (see step 3 above)

3. Set the webhook URL:
   ```bash
   curl -X POST "https://api.telegram.org/bot<YOUR_BOT_TOKEN>/setWebhook?url=https://<worker-name>.<your-subdomain>.workers.dev"
   ```

### Usage

1. Open Yonder app
2. Go to current month balance at the top
3. Breakdown
4. Download CSV
5. Share on Telegarm
6. Choose the chat with your Bot
7. Send the file

## Usage: iOS Shortcuts Webhook

### Setup

1. Set `WEBHOOK_API_KEY` secret (see step 3 above)
2. Get your worker URL: `https://<worker-name>.<your-subdomain>.workers.dev`

### Install iOS shortcut

1. Open the shortcut link from your iPhone: https://www.icloud.com/shortcuts/57b9b3c3c8ac4f98992027bc106ce47f
2. Replace the worker domain in step 2 with your domain
3. Click on "Information" icon at the bottom
4. Enable "Show in Share Sheet"

### Usage

1. Open Yonder app
2. Go to current month balance at the top
3. Breakdown
4. Download CSV
5. Share with "Yonder YNAB"
7. Optionally jump to YNAB app by clicking "OK"

## Yonder CSV Format

The service expects CSV files exported from the Yonder app with this format:

```csv
"Date/Time of transaction","Description","Amount (GBP)","Amount (in Charged Currency)","Currency","Category","Debit or Credit","Country"
"2026-01-01T10:34:50.211697","TFL - Transport for London","3.00","3.00","GBP","Transport","Debit","GBR"
```

See `yonder.csv` in this repository for a sample file.

## Environment Variables Reference

| Variable | Required For | Description |
|----------|-------------|-------------|
| `YNAB_API_KEY` | Both | YNAB Personal Access Token |
| `YNAB_BUDGET_ID` | Both | Target budget UUID (or "last-used") |
| `YNAB_ACCOUNT_ID` | Both | Target account UUID |
| `API_KEY` | Telegram only | Telegram bot token from BotFather |
| `WEBHOOK_API_KEY` | Webhook only | Secret key for iOS Shortcuts authentication |
