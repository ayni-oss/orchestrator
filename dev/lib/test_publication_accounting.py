"""Retained fixtures must be reusable without weakening exactly-once checks."""
import copy
import unittest

from publication_accounting import verify


def pull(number, branch, sha):
    return {'number': number, 'head': {'ref': branch, 'sha': sha},
            'draft': True, 'state': 'open'}


class PublicationAccountingTests(unittest.TestCase):
    def setUp(self):
        self.baseline = {'create_calls': 7, 'pulls': [pull(3, 'old', 'abc')],
                         'references': ['refs/heads/old abc']}
        self.current = copy.deepcopy(self.baseline)
        self.current['create_calls'] += 1
        self.current['pulls'].append(pull(4, 'selected', 'def'))
        self.current['references'].append('refs/heads/selected def')
        self.identity = {'pr_number': 4, 'branch': 'selected', 'commit_sha': 'def'}

    def test_nonempty_baseline_and_selected_identity(self):
        result = verify(self.baseline, self.current, 1, 1, self.identity)
        self.assertEqual(result['create_requests'], 1)
        self.assertEqual(result['total_create_requests'], 8)
        self.assertEqual(result['baseline_retained_prs'], 1)

    def test_empty_baseline(self):
        verify({'create_calls': 0, 'pulls': [], 'references': []}, self.current, 8, 2)

    def test_duplicate_create_rejected(self):
        self.current['create_calls'] += 1
        with self.assertRaisesRegex(ValueError, 'request delta'):
            verify(self.baseline, self.current, 1, 1)

    def test_retained_publication_mutation_rejected(self):
        self.current['pulls'][0]['state'] = 'closed'
        with self.assertRaisesRegex(ValueError, 'previously retained publication'):
            verify(self.baseline, self.current, 1, 1)

    def test_retained_reference_mutation_rejected(self):
        self.current['references'][0] = 'refs/heads/old changed'
        with self.assertRaisesRegex(ValueError, 'Git reference'):
            verify(self.baseline, self.current, 1, 1)

    def test_wrong_selected_sha_rejected(self):
        self.identity['commit_sha'] = 'wrong'
        with self.assertRaisesRegex(ValueError, 'selected draft'):
            verify(self.baseline, self.current, 1, 1, self.identity)

    def test_matching_branch_cannot_reuse_baseline_pull(self):
        with self.assertRaisesRegex(ValueError, 'reused retained identity'):
            verify(self.baseline, self.current, 1, 1,
                   {'pr_number': 3, 'branch': 'old', 'commit_sha': 'abc'})


if __name__ == '__main__':
    unittest.main()
