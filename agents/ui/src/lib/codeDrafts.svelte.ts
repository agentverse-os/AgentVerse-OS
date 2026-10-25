export type DraftImage = { name: string; type: string; data: string; url: string };
type Draft = {
  text: string; images: DraftImage[]; error: string; editing: { seq: number } | null;
  beforeEdit: { text: string; images: DraftImage[] } | null;
  sending: boolean; readingImages: number;
  permissionPending: Record<string, boolean>; permissionSent: Record<string, boolean>; permissionErrors: Record<string, string>;
};
// Attachments stay in memory when the view unmounts; only text goes to localStorage.
// Pending sends and file reads use the same object after navigation.
const drafts = new Map<string, Draft>();
export function codeDraft(key: string): Draft {
  const existing = drafts.get(key); if (existing) return existing;
  let text = ''; try { text = localStorage.getItem(key) ?? ''; } catch {}
  const draft = $state<Draft>({ text, images: [], error: '', editing: null, beforeEdit: null, sending: false, readingImages: 0, permissionPending: {}, permissionSent: {}, permissionErrors: {} });
  // Persist accepted sends even when their original component has already unmounted.
  $effect.root(() => { $effect(() => {
    const value = draft.text;
    try { if (value.trim()) localStorage.setItem(key, value); else localStorage.removeItem(key); } catch {}
  }); });
  drafts.set(key, draft); return draft;
}
