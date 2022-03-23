"""This module contains functionality for resolving log event values using regex lists."""

import errno
from logging import Logger, DEBUG
from multiprocessing import current_process
from os import walk, path, makedirs
from os.path import isdir, realpath, join
from time import time
from typing import List

from hyperscan import Database, HS_FLAG_SINGLEMATCH, HS_FLAG_CASELESS, loadb, dumpb

from logprep.processor.base.exceptions import (NotARulesDirectoryError, InvalidRuleDefinitionError,
                                               InvalidRuleFileError)
from logprep.processor.base.processor import RuleBasedProcessor
from logprep.processor.generic_resolver.rule import GenericResolverRule
from logprep.util.processor_stats import ProcessorStats
from logprep.util.time_measurement import TimeMeasurement


class GenericResolverError(BaseException):
    """Base class for GenericResolver related exceptions."""

    def __init__(self, name: str, message: str):
        super().__init__(f'GenericResolver ({name}): {message}')


class DuplicationError(GenericResolverError):
    """Raise if field already exists."""

    def __init__(self, name: str, skipped_fields: List[str]):
        message = 'The following fields already existed and ' \
                  'were not overwritten by the Generic Resolver: '
        message += ' '.join(skipped_fields)

        super().__init__(name, message)


class GenericResolver(RuleBasedProcessor):
    """Resolve values in documents by referencing a mapping list."""

    def __init__(self, name: str, tree_config: str, hyperscan_db_path: str, logger: Logger):
        super().__init__(name, tree_config, logger)
        self._hyperscan_databases = dict()

        if hyperscan_db_path:
            self._hyperscan_database_path = hyperscan_db_path
        else:
            self._hyperscan_database_path = path.dirname(path.abspath(__file__)) + "/hyperscan_dbs/"
        self.ps = ProcessorStats()

        self._replacements_from_file = {}

    # pylint: disable=arguments-differ
    def add_rules_from_directory(self, rule_paths: List[str]):
        """Add rules from given directory."""
        for path in rule_paths:
            if not isdir(realpath(path)):
                raise NotARulesDirectoryError(self._name, path)

            for root, _, files in walk(path):
                json_files = []
                for file in files:
                    if (file.endswith('.json') or file.endswith('.yml')) and not file.endswith(
                            '_test.json'):
                        json_files.append(file)
                for file in json_files:
                    rules = self._load_rules_from_file(join(root, file))
                    for rule in rules:
                        self._tree.add_rule(rule, self._logger)

        if self._logger.isEnabledFor(DEBUG):
            self._logger.debug(f'{self.describe()} loaded {self._tree.rule_counter} rules '
                               f'({current_process().name})')

        self.ps.setup_rules([None] * self._tree.rule_counter)
    # pylint: enable=arguments-differ

    def _load_rules_from_file(self, path: str):
        try:
            return GenericResolverRule.create_rules_from_file(path)
        except InvalidRuleDefinitionError as error:
            raise InvalidRuleFileError(self._name, path, str(error)) from error

    def describe(self) -> str:
        return f'GenericResolver ({self._name})'

    @TimeMeasurement.measure_time('generic_resolver')
    def process(self, event: dict):
        self._events_processed += 1
        self.ps.update_processed_count(self._events_processed)

        self._event = event

        for rule in self._tree.get_matching_rules(event):
            begin = time()
            self._apply_rules(event, rule)
            processing_time = float('{:.10f}'.format(time() - begin))
            idx = self._tree.get_rule_id(rule)
            self.ps.update_per_rule(idx, processing_time)

    def _apply_rules(self, event, rule):
        conflicting_fields = list()

        hyperscan_db, pattern_id_to_dest_val_map = self._get_hyperscan_database(rule)

        for resolve_source, resolve_target in rule.field_mapping.items():
            keys = resolve_target.split('.')
            src_val = self._get_dotted_field_value(event, resolve_source)

            if src_val:
                result = []

                def on_match(matching_pattern_id: int, fr, to, flags, context):
                    result.append(matching_pattern_id)

                hyperscan_db.scan(src_val, match_event_handler=on_match)

                if result:
                    dict_ = event
                    for idx, key in enumerate(keys):
                        if key not in dict_:
                            if idx == len(keys) - 1:
                                if rule.append_to_list:
                                    dict_[key] = dict_.get('key', [])
                                    dict_[key].append(pattern_id_to_dest_val_map[result[result.index(min(result))]])
                                else:
                                    dict_[key] = pattern_id_to_dest_val_map[result[result.index(min(result))]]
                                break
                            dict_[key] = dict()
                        if isinstance(dict_[key], dict):
                            dict_ = dict_[key]
                        else:
                            if rule.append_to_list and isinstance(dict_[key], list):
                                if pattern_id_to_dest_val_map[result[result.index(min(result))]] not in dict_[key]:
                                    dict_[key].append(pattern_id_to_dest_val_map[result[result.index(min(result))]])
                            else:
                                conflicting_fields.append(keys[idx])

        if conflicting_fields:
            raise DuplicationError(self._name, conflicting_fields)

    def _get_hyperscan_database(self, rule):
        database_id = rule.file_name
        resolve_list = rule.resolve_list

        if database_id not in self._hyperscan_databases.keys():
            try:
                db, value_mapping = self._load_database(database_id, resolve_list)
            except FileNotFoundError:
                db, value_mapping = self._create_database(resolve_list)

                if rule.store_db_persistent:
                    self._save_database(db, database_id)

            self._hyperscan_databases[database_id] = {}
            self._hyperscan_databases[database_id]['db'] = db
            self._hyperscan_databases[database_id]['value_mapping'] = value_mapping

        return self._hyperscan_databases[database_id]['db'], self._hyperscan_databases[database_id]['value_mapping']

    def _load_database(self, database_id, resolve_list):
        value_mapping = {}

        with open(self._hyperscan_database_path + "/" + database_id + '.db', "rb") as f:
            data = f.read()

        for idx, pattern in enumerate(resolve_list.keys()):
            value_mapping[idx] = resolve_list[pattern]

        return loadb(data), value_mapping

    def _save_database(self, database, database_id):
        _create_hyperscan_dbs_dir(self._hyperscan_database_path)
        serialized_db = dumpb(database)

        with open(self._hyperscan_database_path + "/" + database_id + '.db', "wb") as f:
            f.write(serialized_db)

    def _create_database(self, resolve_list):
        database = Database()
        value_mapping = {}
        db_patterns = []

        for idx, pattern in enumerate(resolve_list.keys()):
            db_patterns += [(pattern.encode('utf-8'), idx, HS_FLAG_SINGLEMATCH | HS_FLAG_CASELESS)]
            value_mapping[idx] = resolve_list[pattern]

        if not db_patterns:
            raise GenericResolverError(self._name, 'No patter to compile for hyperscan database!')

        expressions, ids, flags = zip(*db_patterns)
        database.compile(
            expressions=expressions, ids=ids, elements=len(db_patterns), flags=flags
        )

        return database, value_mapping


def _create_hyperscan_dbs_dir(path):
    try:
        makedirs(path)
    except OSError as e:
        if e.errno != errno.EEXIST:
            raise
