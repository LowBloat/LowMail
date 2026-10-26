<script lang="ts">
	import { AlertDialog, Button, Dialog, Row, TextField } from "@codeatlasdev/ui";
	import { focusTextField } from "@codeatlasdev/ui/actions";
	import { dialogs } from "../state/dialogs.svelte";
	import { t } from "../i18n/index.svelte";

	const req = $derived(dialogs.current);
	let value = $state("");
	let input = $state<HTMLInputElement | null>(null);

	$effect(() => {
		const el = input;
		if (el) requestAnimationFrame(() => focusTextField(el));
	});

	$effect(() => {
		if (req?.kind === "prompt") value = req.options.value ?? "";
	});

	function submitPrompt(e?: SubmitEvent) {
		e?.preventDefault();
		if (value.trim()) dialogs.close(value.trim());
	}
</script>

{#if req?.kind === "confirm"}
	<AlertDialog
		open
		title={req.options.title}
		description={req.options.description ?? ""}
		confirmLabel={req.options.confirmLabel ?? t("action.ok")}
		cancelLabel={req.options.cancelLabel ?? t("action.cancel")}
		destructive={req.options.destructive ?? false}
		onconfirm={() => dialogs.close(true)}
		oncancel={() => dialogs.close(false)}
	/>
{:else if req?.kind === "choose"}
	<Dialog open title={req.options.title} description={req.options.description} size="md" onOpenChange={(o) => !o && dialogs.close(null)}>
		{#snippet actions()}
			<Row gap={8} justify="flex-end" wrap>
				{#each req.options.choices as choice (choice.value)}
					<Button variant={choice.variant ?? "outlined"} onclick={() => dialogs.close(choice.value)}>{choice.label}</Button>
				{/each}
			</Row>
		{/snippet}
	</Dialog>
{:else if req?.kind === "prompt"}
	<Dialog open title={req.options.title} description={req.options.description} size="sm" onOpenChange={(o) => !o && dialogs.close(null)}>
		<form id="lm-prompt" onsubmit={submitPrompt}>
			<TextField label={req.options.label} placeholder={req.options.placeholder ?? ""} bind:value bind:ref={input} />
		</form>
		{#snippet actions()}
			<Row gap={8} justify="flex-end">
				<Button variant="ghost" onclick={() => dialogs.close(null)}>{req.options.cancelLabel ?? t("action.cancel")}</Button>
				<Button variant="filled" onclick={() => submitPrompt()} disabled={!value.trim()}>{req.options.confirmLabel ?? t("action.ok")}</Button>
			</Row>
		{/snippet}
	</Dialog>
{/if}
