import { useLayoutEffect, useRef, useState, type FormEvent } from "react";
import { Dialog, DialogHeader } from "@astryxdesign/core/Dialog";
import { Button } from "@astryxdesign/core/Button";
import { HStack, VStack } from "@astryxdesign/core/Stack";
import { Text } from "@astryxdesign/core/Text";
import { TextInput } from "@astryxdesign/core/TextInput";
import { secretsMessage, type SecretProfile, type SecretProfileInput } from "./api";

type Field = Exclude<keyof SecretProfileInput, "provider">;
const defaults: SecretProfileInput = { provider: "vault", name: "", address: "https://", namespace: "", authMount: "userpass", kvMount: "secret", basePath: "" };
const control = /[\u0000-\u001f\u007f-\u009f]/;
function validation(form: SecretProfileInput): Partial<Record<Field, string>> {
  const errors: Partial<Record<Field, string>> = {};
  if (!form.name.trim() || new TextEncoder().encode(form.name).length > 128 || control.test(form.name)) errors.name = "Enter a connection name of at most 128 bytes.";
  try {
    const url = new URL(form.address);
    if (!/^https:\/\/[^/?#\\\s]+\/?$/.test(form.address) || url.protocol !== "https:" || !url.hostname || url.username || url.password || url.pathname !== "/" || url.search || url.hash || form.address.length > 2048) throw Error();
  } catch { errors.address = "Enter an HTTPS origin without credentials, a path, query, or fragment."; }
  for (const field of ["namespace", "authMount", "kvMount", "basePath"] as const) {
    const value = form[field];
    if (!value && (field === "namespace" || field === "basePath")) continue;
    if (!value || new TextEncoder().encode(value).length > 4096 || value.split("/").some(part => !part || part === "." || part === ".." || control.test(part) || /[\\%]/.test(part) || new TextEncoder().encode(part).length > 1024) || (field === "namespace" && /[^\x00-\x7f]/.test(value))) errors[field] = "Use path segments without leading/trailing slashes, dot segments, or encoded characters.";
  }
  return errors;
}
export function ProfileForm({ profile, save, onClose }: { profile?: SecretProfile; save: (input: SecretProfileInput, id?: string) => Promise<SecretProfile>; onClose: (saved?: SecretProfile) => void }) {
  const [form, setForm] = useState<SecretProfileInput>(() => profile ? { provider: profile.provider, name: profile.name, address: profile.address, namespace: profile.namespace, authMount: profile.authMount, kvMount: profile.kvMount, basePath: profile.basePath } : { ...defaults });
  const [touched, setTouched] = useState<Partial<Record<Field, boolean>>>({});
  const [submitted, setSubmitted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fields = useRef<Partial<Record<Field, HTMLInputElement | null>>>({});
  const [focusAttempt, setFocusAttempt] = useState(0);
  const errors = validation(form);
  useLayoutEffect(() => {
    if (!focusAttempt) return;
    const invalid = Object.keys(validation(form))[0];
    if (invalid) fields.current[invalid as Field]?.focus();
  }, [focusAttempt]);
  async function submit(event: FormEvent<HTMLElement>) {
    event.preventDefault();
    if (busy) return;
    setSubmitted(true);
    const invalid = Object.keys(errors)[0];
    if (invalid) {
      setFocusAttempt(value => value + 1);
      return;
    }
    setBusy(true); setError(null);
    try { onClose(await save(form, profile?.id)); }
    catch (reason) { setError(secretsMessage(reason)); setBusy(false); }
  }
  const field = (key: Field, label: string, description?: string, optional = false) => <TextInput ref={element => { fields.current[key] = element; }} htmlName={key} label={label} description={description} value={form[key]} onChange={value => setForm(current => ({ ...current, [key]: value }))} isDisabled={busy} isRequired={!optional} isOptional={optional} hasAutoFocus={key === "name"} autoComplete="off" status={(submitted || touched[key]) && errors[key] ? { type: "error", message: errors[key] } : undefined} statusVariant="detached" />;
  return <Dialog isOpen purpose="form" width="min(36rem, 100%)" onOpenChange={open => { if (!open && !busy) onClose(); }}><DialogHeader title={profile ? "Edit secret connection" : "Add secret connection"} />
    <VStack as="form" gap={4} aria-label={profile ? "Edit secrets connection" : "New secrets connection"} onSubmit={submit} onBlur={event => { if ((event.relatedTarget as HTMLElement | null)?.closest("button")) return; const name = (event.target as HTMLInputElement).name as Field; if (name && !touched[name]) setTouched(current => ({ ...current, [name]: true })); }} style={{ minWidth: 0 }}>
      <Text type="supporting">Vault KV v2. Connection metadata is saved locally; sign in separately.</Text>
      {profile && <Text role="note">Saving changes disconnects this connection and forgets its saved login. Sign in again afterwards.</Text>}
      {error && <Text role="alert">{error}</Text>}
      {field("name", "Connection name")}
      {field("address", "HTTPS endpoint")}
      {field("namespace", "Namespace", "Optional Vault namespace.", true)}
      {field("authMount", "Username/password auth mount", "Authentication method path, not your username.")}
      {field("kvMount", "Default KV v2 mount", "Engine path, not your password. Available engines are loaded after sign-in.")}
      {field("basePath", "Base path", "Optional starting folder within an engine.", true)}
      <HStack gap={2} wrap="wrap"><Button label={busy ? "Saving connection…" : "Save connection"} type="submit" variant="primary" isDisabled={busy} /><Button label="Cancel connection" variant="ghost" isDisabled={busy} onClick={() => onClose()} /></HStack>
    </VStack>
  </Dialog>;
}
