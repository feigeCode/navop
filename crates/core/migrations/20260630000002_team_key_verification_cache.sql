-- Cache team key verification locally so forms can validate user-entered team keys.
ALTER TABLE team_key_cache ADD COLUMN key_verification TEXT;
