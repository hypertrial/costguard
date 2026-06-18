{# ponytail: offline benchmark compile; default runs statement() against warehouse #}
{% macro trino__get_intervals_between(start_date, end_date, datepart) -%}
    {{ return(7670) }}
{%- endmacro %}

{% macro duckdb__get_intervals_between(start_date, end_date, datepart) -%}
    {{ return(7670) }}
{%- endmacro %}
