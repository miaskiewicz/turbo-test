import { entity, relation } from './metadata';
import { Customer } from './customer';
@entity
export class PartnerProfile {
  @relation(() => Customer)
  declare customer: Customer;
}
