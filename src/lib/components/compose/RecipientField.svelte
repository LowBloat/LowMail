<script lang="ts">
	/** Address chips with suggestions from contacts and past recipients. */
	import { X } from "@lucide/svelte";
	import { call } from "../../api";
	import { displayName, formatAddress, parseAddresses } from "../../format";
	import { t } from "../../i18n/index.svelte";
	import type { Address } from "../../types";

	let { label, addresses = $bindable([]), autofocus = false }: { label: string; addresses: Address[]; autofocus?: boolean } = $props();

	let text = $state("");
	let input: HTMLInputElement | undefined = $state();
	let suggestions = $state<Address[]>([]);
	let active = $state(0);
	let invalid = $state<string[]>([]);
	let timer: ReturnType<typeof setTimeout> | undefined;
	let seq = 0;
	const id = `lm-rcpt-${Math.random().toString(36).slice(2)}`;

	$effect(() => {
		if (autofocus && input) requestAnimationFrame(() => input?.focus());
	});

	function commit(raw = text) {
		const { valid, invalid: bad } = parseAddresses(raw);
		for (const a of valid) {
			if (!addresses.some((x) => x.email.toLowerCase() === a.email.toLowerCase())) addresses = [...addresses, a];
		}
		invalid = bad;
		text = bad.join(", ");
		suggestions = [];
	}

	async function pick(a: Address) {
		if (a.email.startsWith("group:")) {
			// A contact group: add every member.
			const members = await call<Address[]>("contacts.groupMembers", { group: a.email.slice(6) }).catch(() => [] as Address[]);
			const fresh = members.filter((m) => !addresses.some((x) => x.email.toLowerCase() === m.email.toLowerCase()));
			addresses = [...addresses, ...fresh];
		} else if (!addresses.some((x) => x.email.toLowerCase() === a.email.toLowerCase())) addresses = [...addresses, a];
		text = "";
		suggestions = [];
		invalid = [];
		input?.focus();
	}

	function remove(a: Address) {
		addresses = addresses.filter((x) => x !== a);
	}

	function edit(a: Address) {
		remove(a);
		text = formatAddress(a);
		input?.focus();
	}

	function onInput() {
		invalid = [];
		clearTimeout(timer);
		const q = text.trim();
		if (q.includes(",") || q.includes(";")) {
			commit();
			return;
		}
		if (q.length < 2) {
			suggestions = [];
			return;
		}
		const mine = ++seq;
		timer = setTimeout(async () => {
			try {
				const found = await call<Address[]>("compose.suggest", { query: q });
				if (mine !== seq) return;
				suggestions = found.filter((s) => !addresses.some((a) => a.email.toLowerCase() === s.email.toLowerCase()));
				active = 0;
			} catch {
				suggestions = [];
			}
		}, 150);
	}

	function onKeydown(e: KeyboardEvent) {
		if (suggestions.length) {
			if (e.key === "ArrowDown") return e.preventDefault(), (active = (active + 1) % suggestions.length);
			if (e.key === "ArrowUp") return e.preventDefault(), (active = (active - 1 + suggestions.length) % suggestions.length);
			if (e.key === "Enter" || e.key === "Tab") {
				e.preventDefault();
				void pick(suggestions[active]);
				return;
			}
			if (e.key === "Escape") {
				e.stopPropagation();
				suggestions = [];
				return;
			}
		}
		if ((e.key === "Enter" || e.key === "," || e.key === ";") && text.trim()) {
			e.preventDefault();
			commit();
		} else if (e.key === "Backspace" && !text && addresses.length) {
			e.preventDefault();
			edit(addresses[addresses.length - 1]);
		}
	}

	function onPaste(e: ClipboardEvent) {
		const pasted = e.clipboardData?.getData("text") ?? "";
		if (/[,;\n]/.test(pasted)) {
			e.preventDefault();
			commit(text + pasted);
		}
	}
</script>

<div class="lm-rcpt" class:lm-rcpt--invalid={invalid.length > 0}>
	<label class="lm-rcpt__label" for={id}>{label}</label>
	<div class="lm-rcpt__box" role="presentation" onclick={() => input?.focus()}>
		{#each addresses as a (a.email)}
			<span class="lm-chip" title={a.email}>
				<button type="button" class="lm-chip__text" onclick={(e) => (e.stopPropagation(), edit(a))}>{displayName(a)}</button>
				<button type="button" class="lm-chip__x" aria-label={t("compose.removeRecipient", { name: displayName(a) })} onclick={(e) => (e.stopPropagation(), remove(a))}><X size={12} /></button>
			</span>
		{/each}
		<input
			{id}
			bind:this={input}
			bind:value={text}
			type="text"
			autocomplete="off"
			spellcheck="false"
			role="combobox"
			aria-expanded={suggestions.length > 0}
			aria-controls="{id}-list"
			aria-autocomplete="list"
			oninput={onInput}
			onkeydown={onKeydown}
			onpaste={onPaste}
			onblur={() => setTimeout(() => (text.trim() ? commit() : (suggestions = [])), 150)}
		/>
	</div>
	{#if suggestions.length}
		<ul class="lm-rcpt__list" id="{id}-list" role="listbox">
			{#each suggestions as s, i (s.email)}
				<li role="option" aria-selected={i === active}>
					<button type="button" class:lm-rcpt__opt--active={i === active} onmousedown={(e) => (e.preventDefault(), void pick(s))}>
						{#if s.email.startsWith("group:")}
							<span class="lm-ellipsis">{t("compose.group", { name: s.name ?? "" })}</span>
							<span class="lm-hint">{t("compose.groupHint")}</span>
						{:else}
							<span class="lm-ellipsis">{s.name || s.email}</span>
							{#if s.name}<span class="lm-hint lm-ellipsis">{s.email}</span>{/if}
						{/if}
					</button>
				</li>
			{/each}
		</ul>
	{/if}
</div>
{#if invalid.length}
	<p class="lm-rcpt__error" role="alert">{t("compose.invalidAddress", { list: invalid.join(", ") })}</p>
{/if}

<style>
	.lm-rcpt {
		position: relative;
		display: flex;
		align-items: flex-start;
		gap: 8px;
		min-height: 38px;
		border-bottom: 1px solid var(--ca-outline-subtle);
	}
	.lm-rcpt--invalid {
		border-bottom-color: var(--lm-danger);
	}
	.lm-rcpt__label {
		flex: 0 0 64px;
		padding-top: 10px;
		color: var(--ca-text-secondary);
		font-size: 13px;
	}
	.lm-rcpt__box {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px;
		padding: 5px 0;
		cursor: text;
	}
	.lm-rcpt__box input {
		flex: 1;
		min-width: 140px;
		height: 26px;
		border: 0;
		outline: none;
		background: transparent;
		color: var(--ca-text-primary);
		font: inherit;
	}
	.lm-chip {
		display: inline-flex;
		align-items: center;
		max-width: 100%;
		height: 26px;
		border: 1px solid var(--ca-outline-default);
		background: var(--ca-surface-sunken);
	}
	.lm-chip__text {
		max-width: 260px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		padding: 0 4px 0 8px;
		border: 0;
		background: transparent;
		color: var(--ca-text-primary);
		font: inherit;
		font-size: 13px;
		cursor: pointer;
	}
	.lm-chip__x {
		display: grid;
		place-items: center;
		width: 22px;
		height: 24px;
		border: 0;
		background: transparent;
		color: var(--ca-text-secondary);
		cursor: pointer;
	}
	.lm-chip__x:hover {
		color: var(--lm-danger);
	}
	.lm-rcpt__list {
		position: absolute;
		left: 72px;
		top: 100%;
		z-index: 20;
		min-width: 320px;
		max-width: calc(100% - 72px);
		margin: 0;
		padding: 4px 0;
		list-style: none;
		background: var(--ca-surface-raised, var(--ca-surface-base));
		border: 1px solid var(--ca-outline-strong);
		box-shadow: 4px 4px 0 color-mix(in oklab, var(--ca-text-primary) 12%, transparent);
	}
	.lm-rcpt__list button {
		width: 100%;
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		padding: 6px 12px;
		border: 0;
		background: transparent;
		color: var(--ca-text-primary);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.lm-rcpt__opt--active {
		background: var(--lm-row-open) !important;
	}
	.lm-rcpt__error {
		margin: 4px 0 0 72px;
		color: var(--lm-danger);
		font-size: 12px;
	}
</style>
