// reached through `link.js`; its imports resolve from here, its real location
import { neighbor } from './neighbor.js';

export const value = `${neighbor} via ${new URL(import.meta.url).pathname.endsWith('/real/target.js')}`;
