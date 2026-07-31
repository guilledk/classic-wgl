import { Component } from '/classic/ecs.js';
import { registerComponent } from '/classic/registry.js';
import type { IEntity, IAnimation, IComponent, ComponentData } from './types.js';

interface AnimationTarget extends IComponent {
    frame: number;
}

export class Animator extends Component {
    speed: number;
    animation: IAnimation | null;
    counter: number;
    frame: number;
    repeat: boolean;
    _playing: boolean;
    target: AnimationTarget;

    constructor(entity: IEntity, target: string | AnimationTarget, speed: number) {
        super(entity);
        this.speed = speed;

        this.animation = null;
        this.counter = 0.0;
        this.frame = 0;
        this.repeat = false;
        this._playing = false;

        this.target = this.game.getGameObject(target as string) as AnimationTarget;

        entity.registerCall('update', this.update.bind(this));
    }

    dump(): ComponentData {
        const minObj = super.dump();
        minObj.target = this.target.toGameObjectString();
        minObj.speed = this.speed;
        return minObj;
    }

    update(): void {
        if ((this._playing || this.repeat) && this.animation) {
            this.counter += this.game.deltaTime * this.animation.rate * this.speed;

            let intCounter = Math.floor(this.counter);
            if (intCounter >= this.animation.sequence.length) {
                intCounter = 0;
                this.counter = 0;
                this._playing = false;
            }

            this.frame = this.animation.sequence[intCounter];

            if (this.target != null) {
                this.target.frame = this.frame;
            }
        }
    }

    play(animation: IAnimation, repeat: boolean = false): void {
        this.repeat = repeat;
        this._playing = true;
        this.animation = animation;
    }

    stop(): void {
        this._playing = false;
        this.repeat = false;
    }
}

// Register component
registerComponent('Animator', Animator);
