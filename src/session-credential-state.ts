export type RuntimeCredentialInput = {
  password: string | null;
  passphrase: string | null;
  savePassword: boolean;
  savePassphrase: boolean;
};

export type SessionCredentialHandleResponse = {
  credentialHandle: string;
  expiresInMs: number;
};

type BackendInvoker = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

export function runtimeCredentialsForStaging(credentials: RuntimeCredentialInput) {
  return {
    password: credentials.savePassword ? null : credentials.password || null,
    passphrase: credentials.savePassphrase ? null : credentials.passphrase || null,
  };
}

export async function stageConnectionCredentials(
  invokeBackend: BackendInvoker,
  sessionId: string,
  credentials: RuntimeCredentialInput,
): Promise<string | null> {
  const { password, passphrase } = runtimeCredentialsForStaging(credentials);
  if (!password && !passphrase) return null;
  const response = await invokeBackend<SessionCredentialHandleResponse>(
    "stage_session_credentials",
    { request: { sessionId, password, passphrase } },
  );
  return response.credentialHandle;
}
