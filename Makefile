.PHONY: dev dev-status

dev:
	pnpm install --frozen-lockfile
	dev/ctl live

dev-status:
	dev/ctl live status
