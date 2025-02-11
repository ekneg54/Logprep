#!/usr/bin/python3
import pytest

from tests.acceptance.util import *
from logprep.util.json_handling import dump_config_as_file, parse_jsonl

basicConfig(level=DEBUG, format="%(asctime)-15s %(name)-5s %(levelname)-8s: %(message)s")
logger = getLogger("Logprep-Test")


@pytest.fixture
def config_template():
    config_yml = {
        "process_count": 1,
        "print_processed_period": 600,
        "timeout": 0.1,
        "profile_pipelines": True,
        "pipeline": [
            {
                "labelername": {
                    "type": "labeler",
                    "schema": "",
                    "include_parent_labels": True,
                    "rules": None,
                }
            }
        ],
        "connector": {
            "type": "writer",
            "output_path": "tests/testdata/acceptance/test_kafka_data_processing_acceptance.out",
            "input_path": "tests/testdata/input_logdata/kafka_raw_event.jsonl",
        },
    }
    return config_yml


@pytest.mark.parametrize(
    "rules, schema, expected_output",
    [
        (
            ["acceptance/labeler/rules_static/rules"],
            "acceptance/labeler/rules_static/labeling/schema.json",
            "labeled_win_event_log.jsonl",
        ),
        (
            [
                "acceptance/labeler/rules_static/rules",
                "acceptance/labeler/rules_static_only_regex/rules",
            ],
            "acceptance/labeler/rules_static_only_regex/labeling/schema.json",
            "labeled_win_event_log_with_regex.jsonl",
        ),
    ],
)
def test_events_labeled_correctly(tmp_path, config_template, rules, schema, expected_output):
    expected_output_path = path.join("tests/testdata/acceptance/expected_result", expected_output)

    set_config(config_template, rules, schema)
    config_path = str(tmp_path / "generated_config.yml")
    dump_config_as_file(config_path, config_template)

    test_output = get_test_output(config_path)
    store_latest_test_output(expected_output, test_output)

    expected_output = parse_jsonl(expected_output_path)

    result = get_difference(test_output, expected_output)

    assert (
        result["difference"][0] == result["difference"][1]
    ), "Missmatch in event at line {}!".format(result["event_line_no"])


def set_config(config_template, rules, schema):
    config_template["pipeline"][0]["labelername"]["schema"] = path.join("tests/testdata", schema)
    config_template["pipeline"][0]["labelername"]["rules"] = [
        path.join("tests/testdata", rule) for rule in rules
    ]
