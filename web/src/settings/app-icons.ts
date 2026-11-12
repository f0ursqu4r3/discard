import classic from '@app-icons/classic.png'
import landscape from '@app-icons/landscape.png'
import graffiti from '@app-icons/graffiti.png'
import manga from '@app-icons/manga.png'
import cyberpunk from '@app-icons/cyberpunk.png'
import matrix from '@app-icons/matrix.png'
import synthwave from '@app-icons/synthwave.png'

export interface AppIconChoice {
  label: string
  src: string
}

/** The app icons the picker can show, keyed by the id the settings file uses. */
export const appIcons: Record<string, AppIconChoice> = {
  classic: { label: 'Classic', src: classic },
  landscape: { label: 'Landscape', src: landscape },
  graffiti: { label: 'Graffiti', src: graffiti },
  manga: { label: 'Manga', src: manga },
  cyberpunk: { label: 'Cyberpunk', src: cyberpunk },
  matrix: { label: 'Matrix', src: matrix },
  synthwave: { label: 'Synthwave', src: synthwave },
}
