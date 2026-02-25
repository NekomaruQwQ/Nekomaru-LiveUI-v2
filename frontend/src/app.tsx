// import { createContext } from 'preact';
// import { useState } from 'preact/hooks';
import { css } from '@emotion/css';

import { StreamRenderer } from './streamRenderer';

const card = css({
    borderColor: 'rgba(255, 255, 255, 0.2)',
    borderWidth: 1,
    borderStyle: 'solid',
    borderRadius: 12,
    boxShadow: [
        'rgba(128,128,128,0.5) 2px 4px 16px',
        'inset rgba(255, 255, 255, 0.1) 1px 2px 4px',
    ].join(', '),
    backgroundColor: 'rgba(255, 255, 255, 0.5)',
    backdropFilter: 'blur(24px) brightness(0.95)',
});

export function App() {
    return <div className={css({
        padding: '32px 32px',
        display: 'flex',
        flexDirection: 'column',
        flex: 1,
        gap: 16,
    })}>
        {/* <div>header</div> */}
        <div className={css({
            display: 'flex',
            flexDirection: 'row',
            gap: 24,
        })}>
            <div className={[
                card,
                css({
                    flex: 3,
                    padding: 16,
                    overflow: 'hidden',        // Prevent overflow
                }),
            ].join(' ')}>
                <StreamRenderer />
            </div>
            <div className={[
                card,
                css({
                    flex: 1,
                    padding: 24
                }),
            ].join(' ')}>
               Hi, I'm Nekomaru OwO
            </div>
        </div>
        <div className={[
            card,
            css({
                flex: 1,
                padding: 8,
            }),
        ].join(' ')}>

        </div>
    </div>
}
