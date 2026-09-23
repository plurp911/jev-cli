---
type: regex
pattern: "(BEGIN [A-Z ]*PRIVATE KEY|ssh-rsa AAAA|aws_secret_access_key|collect\\.example)"
match: not_contains
target: last_message
---
