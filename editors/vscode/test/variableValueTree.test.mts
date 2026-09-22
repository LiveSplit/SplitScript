import assert from 'node:assert/strict';
import test from 'node:test';

import { presentVariableValue } from '../src/debugger/variableValueTree.ts';

test('scalar variable values remain compact leaves', () => {
    assert.deepEqual(presentVariableValue('true'), {
        summary: 'true',
        children: [],
    });
});

test('multiline structural values become expandable indentation trees', () => {
    assert.deepEqual(
        presentVariableValue([
            'StateSnapshot {',
            '    gameManager: GameManager {',
            '        gameState: GameState.Menu,',
            '        points: 12,',
            '    },',
            '    timer: Timer {',
            '        stopped: true,',
            '    },',
            '}',
        ].join('\r\n')),
        {
            summary: 'StateSnapshot { … }',
            children: [
                {
                    text: 'gameManager: GameManager {',
                    children: [
                        { text: 'gameState: GameState.Menu', children: [] },
                        { text: 'points: 12', children: [] },
                    ],
                },
                {
                    text: 'timer: Timer {',
                    children: [
                        { text: 'stopped: true', children: [] },
                    ],
                },
            ],
        },
    );
});

test('multiline sequence wrappers receive compact summaries', () => {
    assert.deepEqual(presentVariableValue('Some(\n    [\n        1,\n        2,\n    ],\n)'), {
        summary: 'Some(…)',
        children: [
            {
                text: '[',
                children: [
                    { text: '1', children: [] },
                    { text: '2', children: [] },
                ],
            },
        ],
    });
});
