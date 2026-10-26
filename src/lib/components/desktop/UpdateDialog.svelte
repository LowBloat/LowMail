<script lang="ts">
	/** A new version with its release notes; installs and restarts when it can. */
	import { Alert, Button, Dialog, Progress } from "@codeatlasdev/ui";
	import { Download, RefreshCw } from "@lucide/svelte";
	import { compose } from "../../state/compose.svelte";
	import { updates } from "../../state/updates.svelte";
	import { t } from "../../i18n/index.svelte";

	/** Release notes are Markdown: headings, bullets, bold and code only. */
	interface Block {
		kind: "h" | "li" | "p";
		parts: { text: string; style: "plain" | "bold" | "code" }[];
	}

	function inline(text: string): Block["parts"] {
		const parts: Block["parts"] = [];
		let last = 0;
		for (const m of text.matchAll(/(\*\*[^*]+\*\*|`[^`]+`)/g)) {
			if (m.index! > last) parts.push({ text: text.slice(last, m.index), style: "plain" });
			const token = m[0];
			parts.push(token.startsWith("**") ? { text: token.slice(2, -2), style: "bold" } : { text: token.slice(1, -1), style: "code" });
			last = m.index! + token.length;
		}
		if (last < text.length) parts.push({ text: text.slice(last), style: "plain" });
		return parts;
	}

	const blocks = $derived.by<Block[]>(() =>
		(updates.available?.notes ?? "")
			.split(/\r?\n/)
			.map((l) => l.trim())
			.filter(Boolean)
			.map((l) => {
				if (l.startsWith("#")) return { kind: "h", parts: inline(l.replace(/^#+\s*/, "")) };
				if (/^[-*] /.test(l)) return { kind: "li", parts: inline(l.slice(2)) };
				return { kind: "p", parts: inline(l) };
			}),
	);
</script>

{#if updates.dialogOpen && updates.available}
	<Dialog
		open={true}
		size="lg"
		title={t("updates.title", { version: updates.available.version })}
		description={updates.currentVersion ? t("updates.current", { version: updates.currentVersion }) : undefined}
		onOpenChange={(o) => {
			if (!o && !updates.installing) updates.dialogOpen = false;
		}}
	>
		<div class="lm-update">
			{#if blocks.length}
				<div class="lm-update__notes">
					{#each blocks as b, i (i)}
						<svelte:element this={b.kind === "h" ? "h3" : "p"} class="lm-update__{b.kind}">
							{#each b.parts as part, j (j)}
								{#if part.style === "bold"}<strong>{part.text}</strong>
								{:else if part.style === "code"}<code>{part.text}</code>
								{:else}{part.text}{/if}
							{/each}
						</svelte:element>
					{/each}
				</div>
			{/if}
			{#if updates.manualHint}
				<Alert severity="info">{updates.manualHint}</Alert>
			{:else if compose.active && !updates.installing}
				<Alert severity="warning">{t("updates.draftOpen")}</Alert>
			{/if}
			{#if updates.installing}
				<Progress
					value={updates.progress === null ? undefined : Math.round(updates.progress * 100)}
					indeterminate={updates.progress === null}
					label={updates.progress === 1 ? t("updates.installing") : t("updates.downloading")}
				/>
			{/if}
			{#if updates.error}
				<Alert severity="error">{updates.error}</Alert>
			{/if}
		</div>
		{#snippet actions()}
			<Button variant="outlined" disabled={updates.installing} onclick={() => (updates.dialogOpen = false)}>{t("updates.later")}</Button>
			{#if updates.policy?.canInstall}
				<Button variant="filled" loading={updates.installing} onclick={() => updates.install()}>
					{#snippet leading()}<RefreshCw size={15} />{/snippet}
					{t("updates.install")}
				</Button>
			{:else}
				<Button variant="filled" onclick={() => updates.download()}>
					{#snippet leading()}<Download size={15} />{/snippet}
					{t("updates.download")}
				</Button>
			{/if}
		{/snippet}
	</Dialog>
{/if}

<style>
	.lm-update {
		display: flex;
		flex-direction: column;
		gap: var(--ca-space-4);
	}
	.lm-update__notes {
		max-height: var(--lm-notes-max-h);
		overflow: auto;
		padding: var(--ca-space-3) var(--ca-space-4);
		border: 1px solid var(--ca-outline-subtle);
		background: var(--ca-surface-sunken);
		line-height: 1.5;
	}
	.lm-update__notes :global(.lm-update__h) {
		margin: var(--ca-space-3) 0 var(--ca-space-1);
		font-size: var(--ca-font-size-body1);
		font-weight: 700;
	}
	.lm-update__notes :global(.lm-update__h:first-child) {
		margin-top: 0;
	}
	.lm-update__notes :global(.lm-update__li) {
		margin: 0 0 var(--ca-space-1) var(--ca-space-4);
		text-indent: calc(-1 * var(--ca-space-3));
	}
	.lm-update__notes :global(.lm-update__li)::before {
		content: "• ";
	}
	.lm-update__notes :global(.lm-update__p) {
		margin: 0 0 var(--ca-space-2);
	}
	.lm-update__notes :global(code) {
		padding: 0 var(--ca-space-1);
		background: var(--ca-surface-base);
		font-family: var(--ca-font-mono);
		font-size: var(--ca-font-size-caption);
	}
</style>
