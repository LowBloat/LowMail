/** Server-side filters and the vacation reply (ManageSieve). */
import { toast } from "@codeatlasdev/ui";
import { call } from "../api";
import { errorText, t } from "../i18n/index.svelte";
import type { FilterState, Filters } from "../types";

class FiltersStore {
	state = $state<FilterState | null>(null);
	loading = $state(false);
	saving = $state(false);
	error = $state<string | null>(null);

	async load(force = false) {
		if (this.state && !force) return;
		this.loading = true;
		this.error = null;
		try {
			this.state = await call<FilterState>("filters.get");
		} catch (e) {
			this.error = errorText(e);
		} finally {
			this.loading = false;
		}
	}

	async save(filters: Filters): Promise<boolean> {
		this.saving = true;
		try {
			await call("filters.save", { filters });
			if (this.state) {
				this.state.filters = filters;
				this.state.foreignScript = null;
				this.state.foreignContent = null;
			}
			toast.success(t("filters.saved"));
			return true;
		} catch (e) {
			toast.error(errorText(e), { duration: 10_000 });
			return false;
		} finally {
			this.saving = false;
		}
	}
}

export const filters = new FiltersStore();
