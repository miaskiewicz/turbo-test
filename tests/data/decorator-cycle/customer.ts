import { entity, relation } from './metadata';
import { PartnerProfile } from './partner-profile';
@entity
export class Customer {
  @relation(() => PartnerProfile)
  declare partnerProfile: PartnerProfile;
}
