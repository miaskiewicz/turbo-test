import { entity, relation } from './metadata';
import { Role } from './role';
@entity
export class PersonRole {
  @relation(() => Role)
  declare role: Role;
}
