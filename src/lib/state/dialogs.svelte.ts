/**
 * Promise-based confirm, choice and prompt dialogs, rendered once by
 * `DialogHost.svelte`, so actions can ask without owning dialog state.
 * Dismissing a dialog (Escape, outside click) always means "cancel".
 */

export interface ConfirmOptions {
	title: string;
	description?: string;
	confirmLabel?: string;
	cancelLabel?: string;
	destructive?: boolean;
}

export interface Choice {
	value: string;
	label: string;
	variant?: "filled" | "outlined" | "ghost" | "destructive";
}

export interface ChooseOptions {
	title: string;
	description?: string;
	choices: Choice[];
}

export interface PromptOptions extends ConfirmOptions {
	label: string;
	value?: string;
	placeholder?: string;
}

type Request =
	| { kind: "confirm"; options: ConfirmOptions; resolve: (ok: boolean) => void }
	| { kind: "choose"; options: ChooseOptions; resolve: (value: string | null) => void }
	| { kind: "prompt"; options: PromptOptions; resolve: (value: string | null) => void };

class Dialogs {
	current = $state<Request | null>(null);

	private cancelCurrent() {
		const req = this.current;
		this.current = null;
		if (req?.kind === "confirm") req.resolve(false);
		else req?.resolve(null);
	}

	confirm(options: ConfirmOptions): Promise<boolean> {
		this.cancelCurrent();
		return new Promise((resolve) => (this.current = { kind: "confirm", options, resolve }));
	}

	choose(options: ChooseOptions): Promise<string | null> {
		this.cancelCurrent();
		return new Promise((resolve) => (this.current = { kind: "choose", options, resolve }));
	}

	prompt(options: PromptOptions): Promise<string | null> {
		this.cancelCurrent();
		return new Promise((resolve) => (this.current = { kind: "prompt", options, resolve }));
	}

	/** `true`/a string accepts; `null`/`false` cancels. */
	close(result: boolean | string | null) {
		const req = this.current;
		this.current = null;
		if (!req) return;
		if (req.kind === "confirm") req.resolve(result === true);
		else req.resolve(typeof result === "string" ? result : null);
	}
}

export const dialogs = new Dialogs();
export const confirm = (o: ConfirmOptions) => dialogs.confirm(o);
export const choose = (o: ChooseOptions) => dialogs.choose(o);
export const prompt = (o: PromptOptions) => dialogs.prompt(o);
