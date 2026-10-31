<script lang="ts">
	import { onMount } from "svelte";
	import { Spinner, Toaster } from "@codeatlasdev/ui";
	import { desktop, isDesktop, isMobile } from "./lib/api";
	import { app } from "./lib/state/app.svelte";
	import { i18n, t } from "./lib/i18n/index.svelte";
	import Login from "./lib/components/Login.svelte";
	import Shell from "./lib/components/Shell.svelte";
	import DialogHost from "./lib/components/DialogHost.svelte";
	import TitleBar from "./lib/components/desktop/TitleBar.svelte";
	import UpdateDialog from "./lib/components/desktop/UpdateDialog.svelte";
	import { updates } from "./lib/state/updates.svelte";
	import AccountEditor from "./lib/components/desktop/AccountEditor.svelte";
	import PasswordPrompt from "./lib/components/desktop/PasswordPrompt.svelte";
	import { windowFrame } from "./lib/state/window-frame.svelte";

	onMount(() => {
		void app.init().then(() => requestAnimationFrame(() => windowFrame.painted()));
		updates.start();
	});

	// The tray menu lives in the native process; keep it in the page's language.
	$effect(() => {
		if (isDesktop && !isMobile) void desktop.setTrayLanguage(i18n.lang).catch(() => {});
	});
</script>

<div class="lm-root">
	{#if isDesktop && !isMobile}
		<TitleBar />
	{/if}
	<main class="lm-main">
		{#if !app.ready}
			<div class="lm-splash" role="status">
				<Spinner size={28} label={t("app.loading")} />
			</div>
		{:else if app.signedIn}
			<Shell />
		{:else if isDesktop}
			{#if app.passwordFor}
				<PasswordPrompt account={app.passwordFor} />
			{:else}
				<AccountEditor account={app.editingAccount ?? null} first={app.accounts.length === 0} />
			{/if}
		{:else}
			<Login />
		{/if}
	</main>
</div>

{#if isDesktop && app.signedIn && app.editingAccount !== undefined}
	<AccountEditor account={app.editingAccount} dialog />
{/if}

<DialogHost />
{#if updates.supported}
	<UpdateDialog />
{/if}
<Toaster />

<style>
	.lm-root {
		display: flex;
		flex-direction: column;
		height: 100%;
	}
	.lm-main {
		flex: 1;
		min-height: 0;
		display: flex;
		flex-direction: column;
	}
	.lm-splash {
		flex: 1;
		display: grid;
		place-items: center;
	}
</style>
