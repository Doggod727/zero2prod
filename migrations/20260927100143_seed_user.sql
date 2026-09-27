-- Add migration script here
INSERT INTO users(user_id, username, password_hash)
VALUES (
           'e80fa3be-c58e-4e4e-9a12-22353cbe92e9',
        'admin',
        '$argon2id$v=19$m=15000,t=2,p=1$q1H6Ju5jHfZ8HWHl5pMV4w$YxGHUV1cK5SscotzfalcHshRgkBVGcblAFrlzzXEaOk'
       );