# SQLCOST002: Repeated JSON extraction

Detects repeated JSON or semi-structured extraction in one SQL file.

Materialize extracted fields once in staging when the same payload field is reused.
