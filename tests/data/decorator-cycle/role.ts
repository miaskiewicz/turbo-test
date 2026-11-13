import { entity, relation } from './metadata';
import { PersonRole } from './person-role';
@entity
export class Role {
  @relation(() => PersonRole)
  declare assignments: PersonRole[];
}
