import { Entity, Field } from './decorators';
import { User } from './user.model';

@Entity('posts')
export class Post {
  @Field({ type: 'string' })
  declare title: string;

  author(): string {
    return 'author-of:' + User.entityName;
  }
}
