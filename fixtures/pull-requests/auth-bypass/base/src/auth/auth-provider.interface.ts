export interface User {
  id: string;
  role: string;
}

export interface Resource {
  id: string;
}

export interface AuthProvider {
  authorize(user: User, resource: Resource): Promise<boolean>;
}
