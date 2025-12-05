import { createContext } from 'preact';
import { useState } from 'preact/hooks';
import { css } from '@emotion/css';

import {
    FluentProvider,
    Card,
    webDarkTheme,
} from '@fluentui/react-components';

export function App() {
    return <FluentProvider theme={webDarkTheme} className={css({
        padding: '8px',
        display: 'flex',
        flexDirection: 'column',
        flex: 1,
        gap: '8px',
    })}>
        <div>header</div>
        <div className={css({
            display: 'flex',
            flexDirection: 'row',
            flex: 1,
            gap: '8px',
        })}>
            <Card className={css({
                flex: 5,
                borderColor: 'rgba(255, 255, 255, 0.25) !important',
                borderWidth: '1px !important',
                borderStyle: 'solid !important',
                borderRadius: '8px !important',
                backgroundColor: 'black !important',
            })} />
            <div className={css({
                flex: 1,
            })}>
               Hi, I'm Nekomaru OwO
            </div>
        </div>
        <div>footer</div>
    </FluentProvider>
}
