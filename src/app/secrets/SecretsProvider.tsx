import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useRouterState } from "@tanstack/react-router";
import { getWindowLayout } from "../layouts/window-layout";
import {
  hasNativeSecrets, secretsError, secretsMessage, secretsProfilePin, secretsProfileRemove, secretsProfileSave, secretsProfileUpdate, secretsProfiles,
  type SecretProfile, type SecretProfileInput,
} from "./api";

type SecretsProfilesContextValue = {
  profiles: SecretProfile[];
  loaded: boolean;
  loading: boolean;
  error: string | null;
  available: boolean;
  refresh: () => Promise<void>;
  save: (input: SecretProfileInput, id?: string) => Promise<SecretProfile>;
  pin: (id: string, pinned: boolean) => Promise<void>;
  remove: (id: string) => Promise<void>;
};
const emptyContext: SecretsProfilesContextValue = {
  profiles: [], loaded: false, loading: false, error: null, available: false,
  refresh: async () => undefined,
  save: async () => { throw "storeUnavailable"; },
  pin: async () => { throw "storeUnavailable"; },
  remove: async () => { throw "storeUnavailable"; },
};
const SecretsProfilesContext = createContext<SecretsProfilesContextValue>(emptyContext);

export function SecretsProvider({ children }: { children: ReactNode }) {
  const available = hasNativeSecrets();
  const [profiles, setProfiles] = useState<SecretProfile[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  const lifetime = useRef(0);
  const mounted = useRef(false);
  const queue = useRef<Promise<void>>(Promise.resolve());
  const pathname = useRouterState({ select: state => state.location.pathname });

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; lifetime.current++; generation.current++; };
  }, []);

  const refresh = useCallback(async () => {
    if (!available || !mounted.current) return;
    const token = ++generation.current;
    const life = lifetime.current;
    setLoading(true);
    setError(null);
    try {
      await queue.current;
      if (!mounted.current || life !== lifetime.current || token !== generation.current) return;
      const next = await secretsProfiles();
      if (mounted.current && life === lifetime.current && token === generation.current) {
        setProfiles(next);
        setLoaded(true);
      }
    } catch (reason) {
      if (mounted.current && life === lifetime.current && token === generation.current) setError(secretsMessage(reason));
    } finally {
      if (mounted.current && life === lifetime.current && token === generation.current) setLoading(false);
    }
  }, [available]);

  const mutate = useCallback(<T,>(action: () => Promise<T>, apply: (value: T, current: SecretProfile[]) => SecretProfile[]): Promise<T> => {
    if (!available) return Promise.reject("storeUnavailable");
    if (!mounted.current) return Promise.reject("cancelled");
    const token = ++generation.current;
    const life = lifetime.current;
    setLoading(true);
    setError(null);
    // Mutations are ordered so separate rows cannot overwrite one another with older snapshots.
    const pending = queue.current.then(async () => {
      if (!mounted.current || life !== lifetime.current) throw "cancelled";
      const result = await action();
      if (mounted.current && life === lifetime.current) setProfiles(current => apply(result, current));
      return result;
    }).catch(reason => {
      if (mounted.current && life === lifetime.current) setError(secretsMessage(reason));
      throw secretsError(reason);
    }).finally(() => {
      if (mounted.current && life === lifetime.current && token === generation.current) setLoading(false);
    });
    queue.current = pending.then(() => undefined, () => undefined);
    return pending;
  }, [available]);

  const save = useCallback((input: SecretProfileInput, id?: string) => mutate(
    () => id ? secretsProfileUpdate(input, id) : secretsProfileSave(input),
    (saved, current) => current.some(item => item.id === saved.id)
      ? current.map(item => item.id === saved.id ? saved : item)
      : [...current, saved],
  ), [mutate]);
  const pin = useCallback(async (id: string, pinned: boolean) => {
    await mutate(() => secretsProfilePin(id, pinned), (updated, current) => current.map(item => item.id === id ? updated : item));
  }, [mutate]);
  const remove = useCallback(async (id: string) => {
    await mutate(() => secretsProfileRemove(id), (_, current) => current.filter(item => item.id !== id));
  }, [mutate]);

  const shouldLoad = available && (pathname === "/secrets" || pathname.startsWith("/secrets/") || getWindowLayout().platform !== "browser");
  useEffect(() => { if (shouldLoad) void refresh(); }, [shouldLoad, refresh]);

  const value = useMemo(() => ({ profiles, loaded, loading, error, available, refresh, save, pin, remove }), [profiles, loaded, loading, error, available, refresh, save, pin, remove]);
  return <SecretsProfilesContext.Provider value={value}>{children}</SecretsProfilesContext.Provider>;
}

export function useSecretsProfiles(): SecretsProfilesContextValue {
  return useContext(SecretsProfilesContext);
}
