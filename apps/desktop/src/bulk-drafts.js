/** Drafts belong to the editing session, including rows outside the current view. */
export function pendingValues(drafts, asof) {
  return [...drafts].filter(([, d]) => d.value && d.value !== d.original)
    .map(([asset_id, d]) => ({ asset_id, amount: d.value, currency: d.currency, asof }));
}

export function settleValues(state, entries, results) {
  const sent = new Map(entries.map((e) => [e.asset_id, e]));
  for (const result of results) {
    const entry = sent.get(result.asset_id);
    if (!entry) continue;
    const draft = state.drafts.get(result.asset_id);
    if (result.ok) {
      state.saved.set(result.asset_id, entry.amount);
      if (draft?.value === entry.amount) state.drafts.delete(result.asset_id);
      else if (draft) draft.original = entry.amount;
    } else if (draft) draft.error = result.error;
  }
}
