<script lang="ts">
	import { Dialog } from "@codeatlasdev/ui";
	import { SHORTCUTS } from "../shortcuts";
	import { t } from "../i18n/index.svelte";

	let { open = $bindable(false) }: { open?: boolean } = $props();
</script>

<Dialog bind:open title={t("shortcuts.title")} size="md">
	<dl class="lm-keys">
		{#each SHORTCUTS as s (s.label)}
			<dt>
				{#each s.keys as key, i (key)}{#if i > 0}<span class="lm-hint"> / </span>{/if}<kbd>{key}</kbd>{/each}
			</dt>
			<dd>{t(s.label)}</dd>
		{/each}
	</dl>
</Dialog>

<style>
	.lm-keys {
		display: grid;
		grid-template-columns: max-content 1fr;
		gap: 6px 18px;
		margin: 0;
	}
	dt {
		text-align: right;
		white-space: nowrap;
	}
	dd {
		margin: 0;
	}
</style>
