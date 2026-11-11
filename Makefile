.PHONY: dev dev-status

dev:
	pnpm install --frozen-lockfile
	dev/ctl live $(if $(filter 1,$(DEBUG)),--debug,)

dev-status:
	dev/ctl live $(if $(filter 1,$(DEBUG)),--debug,) status
