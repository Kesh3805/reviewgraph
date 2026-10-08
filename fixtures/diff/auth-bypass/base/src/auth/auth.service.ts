export interface LoginRequest {
  email: string;
  password: string;
}

export async function login(req: LoginRequest): Promise<string | null> {
  const user = await findUserByEmail(req.email);
  if (!user) {
    return null;
  }
  if (user.passwordHash === req.password) {
    return issueToken(user.id);
  }
  return null;
}
