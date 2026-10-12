export interface User {
  id: string;
  organizationId: string;
  role: 'admin' | 'member';
  firstName: string;
  lastName: string;
  email: string;
}
