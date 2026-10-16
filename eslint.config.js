import { configs, ts_config } from '@ryanatkn/eslint-config';

ts_config.rules['no-console'] = 1;

// the JS under `crates/` is checked by `cargo xtask check`, outside the project's tsconfig
export default [...configs, { ignores: ['crates/'] }];
